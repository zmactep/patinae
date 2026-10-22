//! Molecular color resolution shared by viewport and headless scene consumers.
//!
//! Combines color values and palettes with atom properties and residue ranks.
//! The color crate remains independent of molecular data and scene policy.

use patinae_color::{scheme_id, Color, ColorValue, NamedPalette, ThemedPalette};
use patinae_mol::{Atom, AtomFlags, ObjectMolecule};
use patinae_settings::Settings;

/// Global defaults for representation colors. Targeted assignments live on atoms.
#[derive(Debug, Clone, Copy, Default)]
pub struct RepColorDefaults {
    pub sphere: ColorValue,
    pub stick: ColorValue,
    pub line: ColorValue,
    pub dot: ColorValue,
    pub cartoon: ColorValue,
    pub cartoon_nucleic: ColorValue,
    pub ribbon: ColorValue,
    pub surface: ColorValue,
    pub mesh: ColorValue,
    pub ellipsoid: ColorValue,
}

impl RepColorDefaults {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            sphere: settings.sphere.color,
            stick: settings.stick.color,
            line: settings.line.color,
            dot: settings.dot.color,
            cartoon: settings.cartoon.color,
            cartoon_nucleic: settings.cartoon.nucleic_acid_color,
            ribbon: settings.ribbon.color,
            surface: settings.surface.color,
            mesh: settings.mesh.color,
            ellipsoid: settings.ellipsoid.color,
        }
    }
}

/// Resolves molecular color values to final RGBA values.
pub struct ColorResolver<'a> {
    named_palette: &'a NamedPalette,
    palette: &'a ThemedPalette,
    default_alpha: f32,
    molecule: Option<&'a ObjectMolecule>,
}

impl<'a> ColorResolver<'a> {
    pub fn new(named_palette: &'a NamedPalette, palette: &'a ThemedPalette) -> Self {
        Self {
            named_palette,
            palette,
            default_alpha: 1.0,
            molecule: None,
        }
    }

    pub fn palette(&self) -> &ThemedPalette {
        self.palette
    }

    pub fn with_alpha(mut self, alpha: f32) -> Self {
        self.default_alpha = alpha;
        self
    }

    /// Use the molecule's shared, lazily populated scheme data.
    pub fn with_molecule(mut self, molecule: &'a ObjectMolecule) -> Self {
        self.molecule = Some(molecule);
        self
    }

    pub fn resolve_atom(&self, atom: &Atom) -> [f32; 4] {
        let color = self.resolve_base(atom);
        color.to_rgba(self.default_alpha)
    }

    pub fn resolve_atom_with_alpha(&self, atom: &Atom, alpha: f32) -> [f32; 4] {
        let color = self.resolve_base(atom);
        color.to_rgba(alpha)
    }

    fn resolve_base(&self, atom: &Atom) -> Color {
        self.resolve_value(
            Self::inherited(atom.repr.colors.base, ColorValue::ELEMENT),
            atom,
        )
    }

    fn resolve_value(&self, color: ColorValue, atom: &Atom) -> Color {
        if let Some([r, g, b]) = color.as_rgb8() {
            return Color::from_rgb8(r, g, b);
        }
        if let Some(id) = color.palette_index() {
            return self.named_palette.get_by_index(id).unwrap_or(Color::WHITE);
        }
        match color.scheme_id() {
            Some(scheme_id::ELEMENT) => self.palette.element.get(atom.element as u8),
            Some(scheme_id::CHAIN) => self.resolve_by_chain(atom),
            Some(scheme_id::SECONDARY_STRUCTURE) => self.palette.ss.get(atom.ss_type as u8),
            Some(scheme_id::B_FACTOR) => self.resolve_by_b_factor(atom),
            Some(scheme_id::RESIDUE_TYPE) => self.resolve_by_residue_type(atom),
            Some(scheme_id::RESIDUE_INDEX) => self.resolve_by_residue_index(atom),
            _ => Color::WHITE,
        }
    }

    fn resolve_by_chain(&self, atom: &Atom) -> Color {
        if atom.state.flags.is_biomolecule() {
            self.palette.chains.get(&atom.residue.chain)
        } else {
            self.palette.element.get(atom.element as u8)
        }
    }

    fn inherited(value: ColorValue, fallback: ColorValue) -> ColorValue {
        if value == ColorValue::INHERIT {
            fallback
        } else {
            value
        }
    }

    fn resolve_layers(
        &self,
        atom: &Atom,
        assigned: ColorValue,
        global: ColorValue,
        automatic: ColorValue,
    ) -> [f32; 4] {
        let value = Self::inherited(
            assigned,
            Self::inherited(global, Self::inherited(atom.repr.colors.base, automatic)),
        );
        self.resolve_value(value, atom).to_rgba(self.default_alpha)
    }

    pub fn resolve_cartoon(&self, atom: &Atom) -> [f32; 4] {
        self.resolve_cartoon_with_default(atom, ColorValue::INHERIT)
    }

    pub fn resolve_rep_color(
        &self,
        atom: &Atom,
        assigned: ColorValue,
        global: ColorValue,
    ) -> [f32; 4] {
        self.resolve_layers(atom, assigned, global, ColorValue::ELEMENT)
    }

    pub fn resolve_cartoon_with_default(&self, atom: &Atom, global: ColorValue) -> [f32; 4] {
        self.resolve_layers(atom, atom.repr.colors.cartoon, global, ColorValue::CHAIN)
    }

    pub fn resolve_cartoon_with_defaults(
        &self,
        atom: &Atom,
        defaults: &RepColorDefaults,
    ) -> [f32; 4] {
        let global = if atom.state.flags.contains(AtomFlags::NUCLEIC) {
            Self::inherited(defaults.cartoon_nucleic, defaults.cartoon)
        } else {
            defaults.cartoon
        };
        self.resolve_cartoon_with_default(atom, global)
    }

    fn resolve_by_residue_type(&self, atom: &Atom) -> Color {
        self.palette
            .residue
            .get(&atom.residue.resn)
            .unwrap_or_else(|| self.palette.element.get(atom.element as u8))
    }

    fn resolve_by_residue_index(&self, atom: &Atom) -> Color {
        // Polymer residues are coloured by per-chain rank (blue to red
        // end-to-end). Non-polymer atoms fall back to element colour so
        // ligands, waters, and ions do not inherit the polymer gradient.
        match self
            .molecule
            .and_then(|mol| mol.residue_ranks().t_for(atom))
        {
            Some(t) => self.palette.spectrum.sample(t),
            None => self.palette.element.get(atom.element as u8),
        }
    }

    fn resolve_by_b_factor(&self, atom: &Atom) -> Color {
        let (min, max) = self
            .molecule
            .map_or((0.0, 100.0), ObjectMolecule::b_factor_range);
        let range = max - min;
        if range <= 0.0 {
            return Color::WHITE;
        }
        let t = ((atom.b_factor - min) / range).clamp(0.0, 1.0);
        self.palette.b_factor.sample(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_mol::Element;

    fn test_resolver() -> (NamedPalette, ThemedPalette) {
        (NamedPalette::default(), ThemedPalette::dark())
    }

    #[test]
    fn inheritance_order_and_automatic_defaults_are_explicit() {
        let (named, palette) = test_resolver();
        let resolver = ColorResolver::new(&named, &palette);
        let mut atom = Atom::new("CA", Element::Carbon);
        atom.set_residue("ALA", 1, "A");
        atom.state.flags = AtomFlags::PROTEIN | AtomFlags::POLYMER;
        assert_eq!(
            resolver.resolve_cartoon(&atom),
            palette.chains.get("A").to_rgba(1.0)
        );
        assert_eq!(
            resolver.resolve_atom(&atom),
            palette.element.get(6).to_rgba(1.0)
        );
        atom.repr.colors.base = ColorValue::rgb(255, 0, 0);
        assert_eq!(resolver.resolve_cartoon(&atom), [1.0, 0.0, 0.0, 1.0]);
        let global = ColorValue::rgb(0, 255, 0);
        assert_eq!(
            resolver.resolve_cartoon_with_default(&atom, global),
            [0.0, 1.0, 0.0, 1.0]
        );
        atom.repr.colors.cartoon = ColorValue::ELEMENT;
        assert_eq!(
            resolver.resolve_cartoon_with_default(&atom, global),
            palette.element.get(6).to_rgba(1.0)
        );
        atom.repr.colors.base = ColorValue::rgb(0, 0, 255);
        assert_eq!(
            resolver.resolve_cartoon_with_default(&atom, global),
            palette.element.get(6).to_rgba(1.0)
        );
        atom.repr.colors.cartoon = ColorValue::INHERIT;
        assert_eq!(
            resolver.resolve_cartoon_with_default(&atom, global),
            [0.0, 1.0, 0.0, 1.0]
        );
        assert_eq!(resolver.resolve_cartoon(&atom), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn test_element_coloring() {
        let (named, palette) = test_resolver();
        let resolver = ColorResolver::new(&named, &palette);

        let mut atom = Atom {
            element: Element::Carbon,
            ..Default::default()
        };
        atom.repr.colors.base = patinae_mol::ColorValue::ELEMENT;

        let color = resolver.resolve_atom(&atom);
        assert!(color[0] > 0.4 && color[0] < 0.7);
        assert!(color[1] > 0.4 && color[1] < 0.7);
        assert!(color[2] > 0.4 && color[2] < 0.7);
        assert_eq!(color[3], 1.0);
    }

    #[test]
    fn test_chain_coloring() {
        use patinae_mol::AtomFlags;

        let (named, palette) = test_resolver();
        let resolver = ColorResolver::new(&named, &palette);

        let mut atom = Atom {
            element: Element::Carbon,
            ..Default::default()
        };
        atom.set_residue("ALA", 1, "A");
        atom.state.flags = AtomFlags::PROTEIN | AtomFlags::POLYMER;
        atom.repr.colors.base = patinae_mol::ColorValue::CHAIN;
        let color_a = resolver.resolve_atom(&atom);
        let expected_a = palette.chains.get("A").to_rgba(1.0);
        assert_eq!(color_a, expected_a);

        atom.set_residue("ALA", 1, "B");
        atom.state.flags = AtomFlags::PROTEIN | AtomFlags::POLYMER;
        let color_b = resolver.resolve_atom(&atom);
        assert_ne!(color_a, color_b);

        let mut lig = Atom {
            element: Element::Carbon,
            ..Default::default()
        };
        lig.set_residue("LIG", 1, "Z");
        lig.state.flags = AtomFlags::ORGANIC;
        lig.repr.colors.base = patinae_mol::ColorValue::CHAIN;
        let lig_color = resolver.resolve_atom(&lig);
        let cpk_c = palette.element.get(Element::Carbon as u8).to_rgba(1.0);
        assert_eq!(lig_color, cpk_c);
    }

    #[test]
    fn test_residue_type_coloring() {
        let (named, palette) = test_resolver();
        let resolver = ColorResolver::new(&named, &palette);

        let mut atom = Atom {
            element: Element::Carbon,
            ..Default::default()
        };
        atom.set_residue("ALA", 1, "A");
        atom.repr.colors.base = patinae_mol::ColorValue::RESIDUE_TYPE;
        let color = resolver.resolve_atom(&atom);
        let expected = patinae_color::RES_HYDROPHOBIC;
        assert_eq!(
            [color[0], color[1], color[2]],
            [expected.r, expected.g, expected.b]
        );

        let mut water = Atom {
            element: Element::Carbon,
            ..Default::default()
        };
        water.set_residue("HOH", 1, "A");
        water.repr.colors.base = patinae_mol::ColorValue::RESIDUE_TYPE;
        let w_color = resolver.resolve_atom(&water);
        let cpk_c = palette.element.get(Element::Carbon as u8);
        assert_eq!(w_color[0], cpk_c.r);
    }

    #[test]
    fn test_residue_index_coloring() {
        use patinae_mol::{AtomFlags, ObjectMolecule};
        use std::sync::Arc;

        let mut mol = ObjectMolecule::new("t");
        for (resn, resv) in &[("ALA", 1), ("ALA", 50), ("ALA", 100)] {
            let mut atom = Atom {
                residue: Arc::new(patinae_mol::AtomResidue::from_parts(
                    "A", *resn, *resv, ' ', "",
                )),
                ..Default::default()
            };
            atom.state.flags = AtomFlags::PROTEIN | AtomFlags::POLYMER;
            atom.repr.colors.base = patinae_mol::ColorValue::RESIDUE_INDEX;
            mol.add_atom(atom);
        }

        let (named, palette) = test_resolver();
        let resolver = ColorResolver::new(&named, &palette).with_molecule(&mol);

        let blue = resolver.resolve_atom(mol.get_atom(patinae_mol::AtomIndex(0)).unwrap());
        assert!(blue[2] > 0.9 && blue[0] < 0.1, "rank 0 should be blue");

        let green = resolver.resolve_atom(mol.get_atom(patinae_mol::AtomIndex(1)).unwrap());
        assert!(green[1] > 0.9, "rank 1 should be green");

        let red = resolver.resolve_atom(mol.get_atom(patinae_mol::AtomIndex(2)).unwrap());
        assert!(red[0] > 0.9 && red[2] < 0.1, "rank 2 should be red");
    }

    #[test]
    fn rgb_palette_and_scheme_payloads_resolve_independently() {
        let (named, palette) = test_resolver();
        let resolver = ColorResolver::new(&named, &palette);
        let mut atom = Atom::new("CA", Element::Carbon);
        atom.state.flags = patinae_mol::AtomFlags::PROTEIN;
        atom.repr.colors.base = ColorValue::rgb(0, 0, 2);
        assert_eq!(resolver.resolve_atom(&atom), [0.0, 0.0, 2.0 / 255.0, 1.0]);
        atom.repr.colors.base = ColorValue::palette(2);
        assert_eq!(resolver.resolve_atom(&atom), [1.0, 0.0, 0.0, 1.0]);
        atom.repr.colors.base = ColorValue::CHAIN;
        assert_eq!(
            resolver.resolve_atom(&atom),
            palette.chains.get("").to_rgba(1.0)
        );
    }

    #[test]
    fn test_cartoon_with_default_priority() {
        let (named, palette) = test_resolver();
        let resolver = ColorResolver::new(&named, &palette);

        let mut atom = Atom {
            element: Element::Carbon,
            ..Default::default()
        };
        atom.repr.colors.base = patinae_mol::ColorValue::ELEMENT;
        atom.repr.colors.cartoon = ColorValue::INHERIT;

        let color = resolver.resolve_cartoon_with_default(&atom, patinae_mol::ColorValue::INHERIT);
        assert!(color[0] > 0.4 && color[0] < 0.7);

        let color =
            resolver.resolve_cartoon_with_default(&atom, patinae_mol::ColorValue::palette(2));
        assert!(color[0] > 0.9);
        assert!(color[1] < 0.1);

        atom.repr.colors.cartoon = patinae_mol::ColorValue::palette(3);
        let color =
            resolver.resolve_cartoon_with_default(&atom, patinae_mol::ColorValue::palette(2));
        assert!(color[1] > 0.9);
        assert!(color[0] < 0.1);

        atom.repr.colors.cartoon = ColorValue::INHERIT;
        let color =
            resolver.resolve_cartoon_with_default(&atom, patinae_mol::ColorValue::palette(2));
        assert!(color[0] > 0.9);

        // Scheme values are explicit settings, not inheritance.
        atom.ss_type = patinae_mol::SecondaryStructure::Helix;
        atom.repr.colors.base = patinae_mol::ColorValue::palette(4);
        assert_eq!(
            resolver.resolve_cartoon_with_default(&atom, ColorValue::SECONDARY_STRUCTURE),
            palette.ss.get(atom.ss_type as u8).to_rgba(1.0)
        );
        assert_eq!(
            resolver.resolve_cartoon_with_default(&atom, ColorValue::INHERIT),
            resolver.resolve_atom(&atom)
        );
    }
}
