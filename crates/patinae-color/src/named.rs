//! Named color registry

use serde::{Deserialize, Serialize};

use crate::constants::*;
use crate::{Color, ColorValue, SCHEMES};
use ahash::AHashMap;

/// Registry of named colors
#[derive(Debug, Serialize, Deserialize)]
pub struct NamedPalette {
    colors: Vec<Color>,
    by_name: AHashMap<String, u32>,
    #[serde(skip)]
    names_by_index: std::sync::OnceLock<Vec<Option<String>>>,
}

impl NamedPalette {
    pub fn new() -> Self {
        let mut registry = NamedPalette {
            colors: Vec::with_capacity(256),
            by_name: AHashMap::new(),
            names_by_index: std::sync::OnceLock::new(),
        };
        registry.register_defaults();
        registry
    }

    fn register_defaults(&mut self) {
        // Basic colors
        self.register("white", WHITE);
        self.register("black", BLACK);
        self.register("red", RED);
        self.register("green", GREEN);
        self.register("blue", BLUE);
        self.register("yellow", YELLOW);
        self.register("cyan", CYAN);
        self.register("magenta", MAGENTA);

        // Gray scale
        self.register("gray", GRAY);
        self.register("grey", GRAY);
        self.register("gray10", GRAY10);
        self.register("gray20", GRAY20);
        self.register("gray30", GRAY30);
        self.register("gray40", GRAY40);
        self.register("gray50", GRAY50);
        self.register("gray60", GRAY60);
        self.register("gray70", GRAY70);
        self.register("gray80", GRAY80);
        self.register("gray90", GRAY90);

        // Element-specific colors
        self.register("carbon", ELEM_C);
        self.register("nitrogen", ELEM_N);
        self.register("oxygen", ELEM_O);
        self.register("hydrogen", ELEM_H);
        self.register("sulfur", ELEM_S);

        // Extended palette
        self.register("orange", ORANGE);
        self.register("pink", PINK);
        self.register("purple", PURPLE);
        self.register("brown", BROWN);
        self.register("salmon", SALMON);
        self.register("lime", LIME);
        self.register("slate", SLATE);
        self.register("hotpink", HOTPINK);
        self.register("teal", TEAL);
        self.register("olive", OLIVE);
        self.register("marine", MARINE);
        self.register("forest", FOREST);
        self.register("firebrick", FIREBRICK);
        self.register("chocolate", CHOCOLATE);
        self.register("wheat", WHEAT);
        self.register("violet", VIOLET);
        self.register("lightblue", LIGHT_BLUE);
        self.register("lightgreen", LIGHT_GREEN);
        self.register("lightorange", LIGHT_ORANGE);
        self.register("lightpink", LIGHT_PINK);
        self.register("palecyan", PALE_CYAN);
        self.register("paleyellow", PALE_YELLOW);
        self.register("palegreen", PALE_GREEN);
        self.register("aquamarine", AQUAMARINE);
        self.register("deepteal", DEEP_TEAL);
        self.register("deeppurple", DEEP_PURPLE);
        self.register("deepolive", DEEP_OLIVE);
        self.register("deepsalmon", DEEP_SALMON);
        self.register("tv_red", TV_RED);
        self.register("tv_green", TV_GREEN);
        self.register("tv_blue", TV_BLUE);
        self.register("tv_yellow", TV_YELLOW);
        self.register("tv_orange", TV_ORANGE);

        // Patinae element colors
        self.register("elem_unknown", ELEM_UNKNOWN);
        self.register("elem_h", ELEM_H);
        self.register("elem_c", ELEM_C);
        self.register("elem_n", ELEM_N);
        self.register("elem_o", ELEM_O);
        self.register("elem_p", ELEM_P);
        self.register("elem_s", ELEM_S);
        self.register("elem_f", ELEM_F);
        self.register("elem_cl", ELEM_CL);
        self.register("elem_br", ELEM_BR);
        self.register("elem_i", ELEM_I);
        self.register("elem_he", ELEM_HE);
        self.register("elem_ne", ELEM_NE);
        self.register("elem_ar", ELEM_AR);
        self.register("elem_li", ELEM_LI);
        self.register("elem_na", ELEM_NA);
        self.register("elem_k", ELEM_K);
        self.register("elem_be", ELEM_BE);
        self.register("elem_mg", ELEM_MG);
        self.register("elem_ca", ELEM_CA);
        self.register("elem_b", ELEM_B);
        self.register("elem_al", ELEM_AL);
        self.register("elem_si", ELEM_SI);
        self.register("elem_fe", ELEM_FE);
        self.register("elem_cu", ELEM_CU);
        self.register("elem_zn", ELEM_ZN);

        // SS colors
        self.register("ss_helix", SS_HELIX);
        self.register("ss_sheet", SS_SHEET);
        self.register("ss_loop", SS_LOOP);
        self.register("ss_nucleic", SS_NUCLEIC);

        // Residue type colors
        self.register("res_hydrophobic", RES_HYDROPHOBIC);
        self.register("res_positive", RES_POSITIVE);
        self.register("res_negative", RES_NEGATIVE);
        self.register("res_polar", RES_POLAR);
        self.register("res_cysteine", RES_CYSTEINE);
        self.register("res_glycine", RES_GLYCINE);

        // Chain color names
        self.register("amber", AMBER);
        self.register("emerald", EMERALD);
        self.register("burgundy", BURGUNDY);
        self.register("deepviolet", DEEP_VIOLET);
        self.register("burntorange", BURNT_ORANGE);
        self.register("terracotta", TERRACOTTA);
    }

    /// Append a palette entry and return its stable 24-bit index.
    ///
    /// # Panics
    /// Panics if all 2^24 palette entries have been allocated.
    pub fn register(&mut self, name: &str, color: Color) -> u32 {
        assert!(
            self.colors.len() <= crate::MAX_COLOR_ID as usize,
            "color palette exhausted its 24-bit index space"
        );
        let index = self.colors.len() as u32;
        self.colors.push(color);
        self.by_name.insert(name.to_lowercase(), index);
        self.names_by_index.take();
        index
    }

    pub fn set(&mut self, name: &str, color: Color) -> u32 {
        let key = name.to_lowercase();
        if let Some(&existing_idx) = self.by_name.get(&key) {
            self.colors[existing_idx as usize] = color;
            existing_idx
        } else {
            self.register(name, color)
        }
    }

    pub fn unregister(&mut self, name: &str) -> bool {
        let removed = self.by_name.remove(&name.to_lowercase()).is_some();
        if removed {
            self.names_by_index.take();
        }
        removed
    }

    pub fn get_by_name(&self, name: &str) -> Option<(u32, Color)> {
        self.by_name
            .get(&name.to_lowercase())
            .map(|&idx| (idx, self.colors[idx as usize]))
    }

    pub fn get_by_index(&self, index: u32) -> Option<Color> {
        self.colors.get(index as usize).copied()
    }

    /// Format a color using current palette names and canonical scheme names.
    ///
    /// RGB and inheritance use their literal notation. Unnamed palette entries,
    /// unknown schemes, and reserved tags retain their explicit numeric notation.
    #[must_use]
    pub fn format_color(&self, value: ColorValue) -> String {
        let name = if let Some(index) = value.palette_index() {
            self.names_by_index
                .get_or_init(|| {
                    let mut names: Vec<Option<String>> = vec![None; self.colors.len()];
                    for (name, &id) in &self.by_name {
                        // Reserved literal/scheme names cannot identify palette entries.
                        if name.parse::<ColorValue>().is_ok() {
                            continue;
                        }
                        if let Some(slot) = names.get_mut(id as usize) {
                            if slot.as_ref().is_none_or(|previous| name < previous) {
                                *slot = Some(name.clone());
                            }
                        }
                    }
                    names
                })
                .get(index as usize)
                .and_then(Option::as_deref)
        } else if let Some(id) = value.scheme_id() {
            SCHEMES
                .iter()
                .find(|scheme| scheme.id == id)
                .and_then(|scheme| scheme.names.first().copied())
        } else {
            None
        };
        name.map(str::to_owned).unwrap_or_else(|| value.to_string())
    }

    /// Parse a color literal, scheme, palette name, or normalized RGB vector.
    ///
    /// # Errors
    /// Rejects unknown names, malformed literals, and non-finite RGB components.
    pub fn parse_color(&self, text: &str) -> Result<ColorValue, &'static str> {
        let text = text.trim();
        if let Ok(value) = text.parse::<ColorValue>() {
            return Ok(value);
        }
        if let Some((index, _)) = self.get_by_name(text) {
            return Ok(ColorValue::palette(index));
        }
        if let Some(inner) = text.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let mut parts = inner.split(',');
            let mut rgb = [0.0; 3];
            for component in &mut rgb {
                *component = parts
                    .next()
                    .ok_or("RGB requires three components")?
                    .trim()
                    .parse::<f32>()
                    .map_err(|_| "invalid RGB component")?;
                if !component.is_finite() {
                    return Err("RGB components must be finite");
                }
                *component = component.clamp(0.0, 1.0);
            }
            if parts.next().is_some() {
                return Err("RGB requires three components");
            }
            let [r, g, b] = Color::new(rgb[0], rgb[1], rgb[2]).to_rgb8();
            return Ok(ColorValue::rgb(r, g, b));
        }
        if let Some(color) = Color::from_hex(text) {
            let [r, g, b] = color.to_rgb8();
            return Ok(ColorValue::rgb(r, g, b));
        }
        Err("unknown color value")
    }

    pub fn len(&self) -> usize {
        self.colors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.by_name.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    pub fn merge_from(&mut self, other: &NamedPalette) -> AHashMap<u32, u32> {
        let mut index_map = AHashMap::new();
        for (name, &old_idx) in &other.by_name {
            let color = other.colors[old_idx as usize];
            let new_idx = if let Some(&existing) = self.by_name.get(name) {
                existing
            } else {
                self.register(name, color)
            };
            index_map.insert(old_idx, new_idx);
        }
        index_map
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, Color)> + '_ {
        self.by_name
            .iter()
            .map(|(name, &idx)| (name.as_str(), self.colors[idx as usize]))
    }
}

impl Default for NamedPalette {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_parser_handles_all_color_input_forms() {
        let mut palette = NamedPalette::new();
        let custom = palette.set("custom", Color::RED);
        for (text, value) in [
            ("custom", ColorValue::palette(custom)),
            ("palette:2", ColorValue::palette(2)),
            ("2", ColorValue::palette(2)),
            ("atomic", ColorValue::ELEMENT),
            ("scheme:16777215", ColorValue::scheme(0xffffff)),
            ("#12abef", ColorValue::rgb(0x12, 0xab, 0xef)),
            ("[1, 0, 0]", ColorValue::rgb(255, 0, 0)),
            ("inherit", ColorValue::INHERIT),
        ] {
            assert_eq!(palette.parse_color(text), Ok(value), "{text}");
        }
        for invalid in ["[NaN, 0, 0]", "[1, 2]", "palette:16777216", "unknown_name"] {
            assert!(palette.parse_color(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn color_output_uses_current_names_and_preserves_unnamed_values() {
        let mut palette = NamedPalette::new();
        let index = palette.set("Custom", Color::RED);
        let value = ColorValue::palette(index);
        assert_eq!(palette.format_color(value), "custom");
        palette.set("Custom", Color::BLUE);
        assert_eq!(palette.format_color(value), "custom");
        palette.unregister("Custom");
        assert_eq!(palette.format_color(value), format!("palette:{index}"));

        // Registering the same name again leaves the old entry unnamed.
        let old = palette.register("duplicate", Color::RED);
        let new = palette.register("duplicate", Color::BLUE);
        assert_eq!(
            palette.format_color(ColorValue::palette(old)),
            format!("palette:{old}")
        );
        assert_eq!(palette.format_color(ColorValue::palette(new)), "duplicate");

        let shadowed = palette.set("ss", Color::RED);
        assert_eq!(
            palette.format_color(ColorValue::palette(shadowed)),
            format!("palette:{shadowed}")
        );

        for scheme in SCHEMES {
            assert_eq!(
                palette.format_color(ColorValue::scheme(scheme.id)),
                scheme.names[0]
            );
        }
        for value in [
            ColorValue::rgb(0x12, 0xab, 0xef),
            ColorValue::INHERIT,
            ColorValue::palette(crate::MAX_COLOR_ID),
            ColorValue::scheme(crate::MAX_COLOR_ID),
            ColorValue::try_from_bits(0xfe12_3456).unwrap(),
        ] {
            let text = palette.format_color(value);
            assert_eq!(text.parse::<ColorValue>().unwrap(), value);
        }
        assert_eq!(
            palette.format_color(ColorValue::rgb(0x12, 0xab, 0xef)),
            "#12abef"
        );
    }

    #[test]
    fn test_named_colors() {
        let colors = NamedPalette::new();
        let (_, red) = colors.get_by_name("red").unwrap();
        assert_eq!(red.r, 1.0);
        assert_eq!(red.g, 0.0);
        assert_eq!(red.b, 0.0);
        let (_, red2) = colors.get_by_name("RED").unwrap();
        assert_eq!(red, red2);
    }

    #[test]
    fn test_set_color_new() {
        let mut colors = NamedPalette::new();
        let idx = colors.set("mywhite", Color::new(1.0, 1.0, 1.0));
        let (found_idx, found_color) = colors.get_by_name("mywhite").unwrap();
        assert_eq!(idx, found_idx);
        assert_eq!(found_color, Color::new(1.0, 1.0, 1.0));
    }

    #[test]
    fn test_set_color_update() {
        let mut colors = NamedPalette::new();
        let idx1 = colors.set("mycolor", Color::new(1.0, 0.0, 0.0));
        let idx2 = colors.set("mycolor", Color::new(0.0, 1.0, 0.0));
        assert_eq!(idx1, idx2);
        let (_, found_color) = colors.get_by_name("mycolor").unwrap();
        assert_eq!(found_color, Color::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn test_unregister_color() {
        let mut colors = NamedPalette::new();
        colors.set("mycolor", Color::new(1.0, 0.0, 0.0));
        assert!(colors.get_by_name("mycolor").is_some());
        assert!(colors.unregister("mycolor"));
        assert!(colors.get_by_name("mycolor").is_none());
    }

    #[test]
    fn test_unregister_nonexistent() {
        let mut colors = NamedPalette::new();
        assert!(!colors.unregister("nonexistent"));
    }
}
