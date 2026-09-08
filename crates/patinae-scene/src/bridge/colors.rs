//! Host color policy for deferred renderer writes and standalone color arrays.

use std::collections::HashMap;
use std::sync::OnceLock;

use patinae_color::{Color as RgbColor, ColorResolver, NamedPalette, ThemedPalette};
use patinae_mol::{
    polymer_residue_ranks, Atom, AtomFlags, AtomIndex, ObjectMolecule, ResidueRankTable,
    COLOR_BY_CHAIN, COLOR_UNSET,
};
use patinae_render::{
    pack_rep_rgb8, AtomColorSource, ColorLutEntry, RepColorLutEntry, REP_COLOR_INHERIT,
};
use patinae_settings::{Color as SettingColor, ResolvedSettings, Settings};

use crate::object::{Object, ObjectRegistry};

/// Resolves one object's colors directly into the renderer's persistent buffer.
pub(super) struct ObjectColorSource<'a> {
    molecule: &'a ObjectMolecule,
    named: &'a NamedPalette,
    themed: &'a ThemedPalette,
    defaults: RepColorDefaults,
    ranks: OnceLock<ResidueRankTable>,
}

impl<'a> ObjectColorSource<'a> {
    pub(super) fn new(
        molecule: &'a ObjectMolecule,
        settings: &ResolvedSettings,
        named: &'a NamedPalette,
        themed: &'a ThemedPalette,
    ) -> Self {
        Self {
            molecule,
            named,
            themed,
            defaults: RepColorDefaults::from_settings(settings),
            ranks: OnceLock::new(),
        }
    }

    fn resolver(&self) -> ColorResolver<'_> {
        ColorResolver::new(self.named, self.themed).with_residue_ranks(Some(
            self.ranks
                .get_or_init(|| polymer_residue_ranks(self.molecule)),
        ))
    }
}

impl AtomColorSource for ObjectColorSource<'_> {
    fn len(&self) -> usize {
        self.molecule.atom_count()
    }

    fn get(&self, index: usize) -> Option<ColorLutEntry> {
        let atom = self
            .molecule
            .get_atom(AtomIndex(u32::try_from(index).ok()?))?;
        let resolver = self.resolver();
        Some(ColorLutEntry::new(
            resolver.resolve_atom(atom),
            resolve_rep_colors(&resolver, atom, &self.defaults),
        ))
    }

    fn write(&self, target: &mut [ColorLutEntry]) {
        assert_eq!(target.len(), self.len());
        // Release bulk-resolution scratch after this object, rather than
        // retaining every dirty object's residue table until the frame ends.
        let ranks = polymer_residue_ranks(self.molecule);
        let resolver = ColorResolver::new(self.named, self.themed).with_residue_ranks(Some(&ranks));
        for (entry, atom) in target.iter_mut().zip(self.molecule.atoms()) {
            *entry = ColorLutEntry::new(
                resolver.resolve_atom(atom),
                resolve_rep_colors(&resolver, atom, &self.defaults),
            );
        }
    }
}

/// Pre-resolved per-atom RGBA per molecule object.
///
/// Standalone callers can retain these arrays and consult
/// [`ResolvedSceneColors::needs_rebuild`]. The normal viewport/capture paths
/// use deferred sources instead, keeping the full color arrays only in the renderer.
#[derive(Default)]
pub struct ResolvedSceneColors {
    by_object: HashMap<String, Vec<[f32; 4]>>,
    rep_by_object: HashMap<String, Vec<RepColorLutEntry>>,
}

/// Resolve a global/object color setting through the active named palette.
/// Non-negative values that are not palette indices are treated as packed
/// `0x00RRGGBB` colors, matching command parsing for `set *_color, [r,g,b]`.
pub fn resolve_setting_color(
    setting: SettingColor,
    named: &NamedPalette,
    fallback: [f32; 4],
) -> [f32; 4] {
    if setting.0 < 0 {
        return fallback;
    }

    let color = named
        .get_by_index(setting.0 as u32)
        .unwrap_or_else(|| RgbColor::from_packed_rgb(setting.0));
    color.to_rgba(fallback[3])
}

impl ResolvedSceneColors {
    /// Resolve every enabled molecule object's atoms to RGBA via the
    /// element / chain / SS / b-factor / residue palettes.
    pub fn build(
        registry: &ObjectRegistry,
        settings: &Settings,
        named: &NamedPalette,
        themed: &ThemedPalette,
    ) -> Self {
        let mut s = Self::default();
        s.rebuild(registry, settings, named, themed);
        s
    }

    /// In-place rebuild — preserves the existing `HashMap` allocation, so
    /// per-frame churn drops to zero on assemblies. Reuses each object's
    /// `Vec<[f32; 4]>` capacity when present.
    pub fn rebuild(
        &mut self,
        registry: &ObjectRegistry,
        settings: &Settings,
        named: &NamedPalette,
        themed: &ThemedPalette,
    ) {
        // Drop entries for objects that are gone / disabled / coord-less so
        // `get` doesn't return stale colours.
        self.by_object.retain(|name, _| {
            registry
                .get_molecule(name)
                .map(|mol_obj| mol_obj.state().enabled && mol_obj.display_coord_set().is_some())
                .unwrap_or(false)
        });
        self.rep_by_object.retain(|name, _| {
            registry
                .get_molecule(name)
                .map(|mol_obj| mol_obj.state().enabled && mol_obj.display_coord_set().is_some())
                .unwrap_or(false)
        });

        for name in registry.names() {
            let Some(mol_obj) = registry.get_molecule(name) else {
                continue;
            };
            if !mol_obj.state().enabled {
                continue;
            }
            let mol = mol_obj.molecule();
            if mol_obj.display_coord_set().is_none() {
                continue;
            }
            let ranks = polymer_residue_ranks(mol);
            let resolver = ColorResolver::new(named, themed).with_residue_ranks(Some(&ranks));
            let resolved = ResolvedSettings::resolve(settings, mol_obj.overrides());
            let defaults = RepColorDefaults::from_settings(&resolved);
            let entry = self.by_object.entry(name.to_string()).or_default();
            entry.clear();
            entry.reserve(mol.atom_count());
            let rep_entry = self.rep_by_object.entry(name.to_string()).or_default();
            rep_entry.clear();
            rep_entry.reserve(mol.atom_count());
            for atom in mol.atoms() {
                let base = resolver.resolve_atom(atom);
                entry.push(base);
                rep_entry.push(resolve_rep_colors(&resolver, atom, &defaults));
            }
        }
    }

    /// Returns `true` iff at least one enabled object has a colour-
    /// affecting dirty bit set, OR the cache is empty / out of sync with
    /// the registry. Hosts call this once per frame and only call
    /// [`Self::rebuild`] when it returns `true`. Camera-rotate frames
    /// (no dirty objects) reuse the cached buffers — saves the per-atom
    /// resolve loop entirely.
    pub fn needs_rebuild(&self, registry: &ObjectRegistry) -> bool {
        let mut alive_count = 0usize;
        for name in registry.names() {
            let Some(mol_obj) = registry.get_molecule(name) else {
                continue;
            };
            if !mol_obj.state().enabled {
                continue;
            }
            if mol_obj.display_coord_set().is_none() {
                continue;
            }
            alive_count += 1;

            // Any dirty bit forces a rebuild. Coord changes can re-alias
            // residues onto a different B-factor / SS so colour
            // resolution depends on them too — we conservatively rebuild
            // on any non-empty `DirtyFlags`.
            if mol_obj.is_dirty() {
                return true;
            }
            // Cache miss for an object that's enabled but never resolved.
            if !self.by_object.contains_key(name) {
                return true;
            }
            if !self.rep_by_object.contains_key(name) {
                return true;
            }
            // Atom count mismatch (e.g. atoms added since last rebuild).
            if let Some(prev) = self.by_object.get(name) {
                if prev.len() != mol_obj.molecule().atom_count() {
                    return true;
                }
            }
            if let Some(prev) = self.rep_by_object.get(name) {
                if prev.len() != mol_obj.molecule().atom_count() {
                    return true;
                }
            }
        }
        // Registry shrunk (object deleted / disabled).
        if self.by_object.len() != alive_count || self.rep_by_object.len() != alive_count {
            return true;
        }
        false
    }

    pub fn get(&self, name: &str) -> Option<&[[f32; 4]]> {
        self.by_object.get(name).map(|v| v.as_slice())
    }

    pub fn get_rep(&self, name: &str) -> Option<&[RepColorLutEntry]> {
        self.rep_by_object.get(name).map(|v| v.as_slice())
    }
}

#[derive(Debug, Clone, Copy)]
struct RepColorDefaults {
    sphere: i32,
    stick: i32,
    line: i32,
    dot: i32,
    cartoon: i32,
    cartoon_nucleic: i32,
    ribbon: i32,
    surface: i32,
    mesh: i32,
    ellipsoid: i32,
}

impl RepColorDefaults {
    fn from_settings(settings: &ResolvedSettings) -> Self {
        Self {
            sphere: settings.sphere.color.0,
            stick: settings.stick.color.0,
            line: settings.line.color.0,
            dot: settings.dot.color.0,
            cartoon: settings.cartoon.color.0,
            cartoon_nucleic: settings.cartoon.nucleic_acid_color.0,
            ribbon: settings.ribbon.color.0,
            surface: settings.surface.color.0,
            mesh: settings.mesh.color.0,
            ellipsoid: settings.ellipsoid.color.0,
        }
    }
}

fn resolve_rep_colors(
    resolver: &ColorResolver<'_>,
    atom: &Atom,
    defaults: &RepColorDefaults,
) -> RepColorLutEntry {
    let colors = &atom.repr.colors;
    RepColorLutEntry {
        sphere: resolve_rep_override(resolver, atom, colors.sphere, defaults.sphere),
        stick: resolve_rep_override(resolver, atom, colors.stick, defaults.stick),
        line: resolve_rep_override(resolver, atom, colors.line, defaults.line),
        dot: resolve_rep_override(resolver, atom, colors.dot, defaults.dot),
        cartoon: resolve_cartoon_override(resolver, atom, colors.cartoon, defaults),
        ribbon: resolve_rep_override(resolver, atom, colors.ribbon, defaults.ribbon),
        surface: resolve_rep_override(resolver, atom, colors.surface, defaults.surface),
        mesh: resolve_rep_override(resolver, atom, colors.mesh, defaults.mesh),
        ellipsoid: resolve_rep_override(resolver, atom, colors.ellipsoid, defaults.ellipsoid),
        _pad0: REP_COLOR_INHERIT,
        _pad1: REP_COLOR_INHERIT,
        _pad2: REP_COLOR_INHERIT,
    }
}

fn resolve_cartoon_override(
    resolver: &ColorResolver<'_>,
    atom: &Atom,
    per_atom_color: i32,
    defaults: &RepColorDefaults,
) -> u32 {
    let explicit_atom_color = per_atom_color != COLOR_UNSET && per_atom_color != COLOR_BY_CHAIN;
    if explicit_atom_color {
        return pack_rep_rgb8(resolver.resolve_rep_color(atom, per_atom_color, defaults.cartoon));
    }

    if atom.state.flags.contains(AtomFlags::NUCLEIC) && defaults.cartoon_nucleic >= 0 {
        return pack_rep_rgb8(resolver.resolve_rep_color(
            atom,
            COLOR_UNSET,
            defaults.cartoon_nucleic,
        ));
    }

    if defaults.cartoon >= 0 {
        return pack_rep_rgb8(resolver.resolve_rep_color(atom, COLOR_UNSET, defaults.cartoon));
    }

    resolve_rep_override(resolver, atom, per_atom_color, defaults.cartoon)
}

fn resolve_rep_override(
    resolver: &ColorResolver<'_>,
    atom: &Atom,
    per_atom_color: i32,
    default_color: i32,
) -> u32 {
    if per_atom_color == COLOR_UNSET && default_color < 0 {
        REP_COLOR_INHERIT
    } else {
        pack_rep_rgb8(resolver.resolve_rep_color(atom, per_atom_color, default_color))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MoleculeObject;
    use lin_alg::f32::Vec3;
    use patinae_mol::{AtomBuilder, Element, MoleculeBuilder};

    fn defaults(cartoon: i32, cartoon_nucleic: i32) -> RepColorDefaults {
        RepColorDefaults {
            sphere: COLOR_UNSET,
            stick: COLOR_UNSET,
            line: COLOR_UNSET,
            dot: COLOR_UNSET,
            cartoon,
            cartoon_nucleic,
            ribbon: COLOR_UNSET,
            surface: COLOR_UNSET,
            mesh: COLOR_UNSET,
            ellipsoid: COLOR_UNSET,
        }
    }

    #[test]
    fn setting_color_resolves_named_palette_then_packed_rgb() {
        let named = NamedPalette::default();
        let (red_idx, red) = named.get_by_name("red").unwrap();
        assert_eq!(
            resolve_setting_color(SettingColor(red_idx as i32), &named, [0.0, 0.0, 0.0, 1.0]),
            red.to_rgba(1.0)
        );
        assert_eq!(
            resolve_setting_color(SettingColor(0x123456), &named, [0.0, 0.0, 0.0, 0.5]),
            RgbColor::from_packed_rgb(0x123456).to_rgba(0.5)
        );
    }

    #[test]
    fn cartoon_nucleic_color_precedes_generic_cartoon_color() {
        let named = NamedPalette::default();
        let themed = ThemedPalette::dark();
        let resolver = ColorResolver::new(&named, &themed);
        let (red_idx, red) = named.get_by_name("red").unwrap();
        let (blue_idx, blue) = named.get_by_name("blue").unwrap();
        let defaults = defaults(blue_idx as i32, red_idx as i32);

        let mut nucleic = Atom::new("P", Element::Phosphorus);
        nucleic.state.flags = AtomFlags::NUCLEIC | AtomFlags::POLYMER;
        nucleic.repr.colors.cartoon = COLOR_BY_CHAIN;
        assert_eq!(
            resolve_cartoon_override(&resolver, &nucleic, nucleic.repr.colors.cartoon, &defaults),
            pack_rep_rgb8(red.to_rgba(1.0))
        );

        let mut protein = Atom::new("CA", Element::Carbon);
        protein.state.flags = AtomFlags::PROTEIN | AtomFlags::POLYMER;
        protein.repr.colors.cartoon = COLOR_BY_CHAIN;
        assert_eq!(
            resolve_cartoon_override(&resolver, &protein, protein.repr.colors.cartoon, &defaults),
            pack_rep_rgb8(blue.to_rgba(1.0))
        );
    }

    fn single_atom_registry(atom: patinae_mol::Atom) -> ObjectRegistry {
        let mol = MoleculeBuilder::new("obj")
            .add_atom(atom, Vec3::new(0.0, 0.0, 0.0))
            .build();
        let mut registry = ObjectRegistry::new();
        registry.add(MoleculeObject::with_name(mol, "obj"));
        registry
    }

    #[test]
    fn bridge_resolves_rep_specific_atom_colors() {
        let named = NamedPalette::new();
        let themed = ThemedPalette::dark();
        let red = named.get_by_name("red").unwrap().0 as i32;
        let green = named.get_by_name("green").unwrap().0 as i32;
        let blue = named.get_by_name("blue").unwrap().0 as i32;
        let yellow = named.get_by_name("yellow").unwrap().0 as i32;
        let cyan = named.get_by_name("cyan").unwrap().0 as i32;
        let magenta = named.get_by_name("magenta").unwrap().0 as i32;
        let white = named.get_by_name("white").unwrap().0 as i32;
        let gray = named.get_by_name("gray").unwrap().0 as i32;
        let black = named.get_by_name("black").unwrap().0 as i32;

        let mut atom = AtomBuilder::new().name("CA").element_symbol("C").build();
        atom.repr.colors.sphere = red;
        atom.repr.colors.stick = green;
        atom.repr.colors.line = blue;
        atom.repr.colors.dot = yellow;
        atom.repr.colors.cartoon = cyan;
        atom.repr.colors.ribbon = magenta;
        atom.repr.colors.surface = white;
        atom.repr.colors.mesh = gray;
        atom.repr.colors.ellipsoid = black;

        let registry = single_atom_registry(atom);
        let colors = ResolvedSceneColors::build(&registry, &Settings::default(), &named, &themed);
        let rep = colors.get_rep("obj").unwrap()[0];
        let expected =
            |idx: i32| pack_rep_rgb8(named.get_by_index(idx as u32).unwrap().to_rgba(1.0));

        assert_eq!(rep.sphere, expected(red));
        assert_eq!(rep.stick, expected(green));
        assert_eq!(rep.line, expected(blue));
        assert_eq!(rep.dot, expected(yellow));
        assert_eq!(rep.cartoon, expected(cyan));
        assert_eq!(rep.ribbon, expected(magenta));
        assert_eq!(rep.surface, expected(white));
        assert_eq!(rep.mesh, expected(gray));
        assert_eq!(rep.ellipsoid, expected(black));
    }

    #[test]
    fn bridge_resolves_global_rep_color_defaults() {
        let named = NamedPalette::new();
        let themed = ThemedPalette::dark();
        let red = named.get_by_name("red").unwrap().0 as i32;

        let atom = AtomBuilder::new().name("CA").element_symbol("C").build();
        let registry = single_atom_registry(atom);
        let mut settings = Settings::default();
        settings.sphere.color = SettingColor(red);

        let colors = ResolvedSceneColors::build(&registry, &settings, &named, &themed);
        let rep = colors.get_rep("obj").unwrap()[0];

        assert_eq!(
            rep.sphere,
            pack_rep_rgb8(named.get_by_index(red as u32).unwrap().to_rgba(1.0))
        );
        assert_eq!(rep.stick, REP_COLOR_INHERIT);
    }
}
