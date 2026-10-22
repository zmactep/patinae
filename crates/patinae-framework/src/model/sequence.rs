//! Sequence Model
//!
//! Pure domain types and logic for the sequence viewer.
//! No egui dependency — testable, serializable, headless-compatible.
//!
//! Contains: sequence cache, domain types (ResidueRef, SeqObject, etc.),
//! and pure transformation functions (build_seq_object, compress_resi_list, etc.).

use std::collections::HashSet;
use std::ops::Range;

use patinae_color::{Color, NamedPalette, ThemedPalette};

/// Everything the model needs from the outside world to rebuild its cache.
///
/// Bundles color-lookup tables so that `rebuild_cache` / `build_seq_object`
/// take a single context reference instead of a growing parameter list.
pub struct SequenceColorContext<'a> {
    pub named_palette: &'a NamedPalette,
    pub palette: &'a ThemedPalette,
    pub settings: &'a patinae_settings::Settings,
}
use patinae_mol::{
    is_capping_group, is_ion, is_standard_amino_acid, is_standard_nucleotide, is_water,
    nucleotide_to_char, residue_to_char, three_to_one, Atom, ObjectMolecule,
};
use patinae_scene::{ColorResolver, Object, ObjectRegistry, RepColorDefaults};

/// Reference to a specific residue in a loaded object
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResidueRef {
    pub object_name: String,
    pub chain_id: String,
    pub resv: i32,
}

/// Semantic classification of a residue for sequence viewer coloring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidueKind {
    /// One of the 20 standard amino acids.
    AminoAcidCanonical,
    /// Known AA variant (HIP, CYX, MSE, SEP, ACE, NME, …)
    AminoAcidNonCanonical,
    /// Standard unmodified nucleotide
    NucleotideCanonical,
    /// Modified or unknown nucleotide
    NucleotideNonCanonical,
    /// Non-polymer HETATM: ligand, lipid, cofactor
    Ligand,
}

impl ResidueKind {
    pub fn is_polymer(self) -> bool {
        !matches!(self, ResidueKind::Ligand)
    }
}

/// Single residue entry in the sequence viewer
#[derive(Debug, Clone)]
pub struct SeqResidue {
    pub chain: String,
    pub resn: String,
    pub resv: i32,
    pub display_char: char,
    /// Full display label: single char for polymer residues, `"[ATP]"` for ligands
    pub display_label: String,
    /// Number of character cells this residue occupies in the sequence row
    pub char_width: usize,
    pub atom_range: Range<usize>,
    /// Pre-resolved color for the default residue coloring mode (RGB).
    /// For canonical amino acids this is the named-color of the representative atom;
    /// for other kinds the render path uses fixed constants instead.
    pub named_color: [u8; 3],
    /// Pre-resolved color matching the 3D viewer (RGB)
    pub viewer_color: [u8; 3],
    /// Drives color and font size in the sequence viewer
    pub kind: ResidueKind,
}

/// Sequence data for one chain
#[derive(Debug, Clone)]
pub struct SeqChain {
    pub chain_id: String,
    pub residues: Vec<SeqResidue>,
}

/// Sequence data for one object
#[derive(Debug, Clone)]
pub struct SeqObject {
    pub object_name: String,
    pub chains: Vec<SeqChain>,
}

/// Sequence panel interaction state (no egui dependency).
///
/// Tracks drag selection and hover state for the sequence viewer.
pub struct SequenceUiState {
    /// Active drag start: (object_index, chain_index, start_residue_index)
    pub drag_start: Option<(usize, usize, usize)>,
    /// Current drag end residue index (updated each frame while dragging)
    pub drag_end: Option<usize>,
    /// Current sequence hover state for change detection
    pub current_hover: Option<ResidueRef>,
    /// When true, residue symbols are colored to match the 3D viewer
    pub viewer_colors: bool,
}

impl Default for SequenceUiState {
    fn default() -> Self {
        Self::new()
    }
}

impl SequenceUiState {
    pub fn new() -> Self {
        Self {
            drag_start: None,
            drag_end: None,
            current_hover: None,
            viewer_colors: false,
        }
    }

    /// Clear any active drag state (called on dock/float transition).
    pub fn clear_drag(&mut self) {
        self.drag_start = None;
        self.drag_end = None;
    }
}

/// Sequence model: cached sequence data and highlighted residues.
pub struct SequenceModel {
    /// Cached sequence data per object
    pub sequences: Vec<SeqObject>,
    /// Number of objects when cache was last built
    cached_object_count: usize,
    /// Number of enabled objects when cache was last built
    cached_enabled_count: usize,
    /// Registry generation when cache was last built
    cached_generation: u64,
    /// Currently highlighted residues from 3D selection
    pub highlighted: HashSet<ResidueRef>,
}

impl Default for SequenceModel {
    fn default() -> Self {
        Self::new()
    }
}

impl SequenceModel {
    pub fn new() -> Self {
        Self {
            sequences: Vec::new(),
            cached_object_count: 0,
            cached_enabled_count: 0,
            cached_generation: 0,
            highlighted: HashSet::new(),
        }
    }

    /// Update the highlighted residues from the current 3D selection.
    pub fn update_highlights(&mut self, highlighted: HashSet<ResidueRef>) {
        self.highlighted = highlighted;
    }

    /// Rebuild the sequence cache from the registry.
    pub fn rebuild_cache(&mut self, registry: &ObjectRegistry, color_ctx: &SequenceColorContext) {
        self.sequences.clear();

        for name in registry.names() {
            if let Some(mol_obj) = registry.get_molecule(name) {
                if !mol_obj.is_enabled() {
                    continue;
                }
                let mol = mol_obj.molecule();
                let seq_obj = build_seq_object(name, mol, color_ctx);
                if !seq_obj.chains.is_empty() {
                    self.sequences.push(seq_obj);
                }
            }
        }

        self.cached_object_count = registry.names().count();
        self.cached_enabled_count = registry.enabled_objects().count();
        self.cached_generation = registry.generation();
    }

    /// Check if cache needs rebuilding.
    ///
    /// When `viewer_colors` is active, dirty molecule flags are enough to detect
    /// color-affecting commands. This keeps camera/hover frames O(objects)
    /// instead of hashing every atom in large assemblies.
    pub fn needs_rebuild(&self, registry: &ObjectRegistry, viewer_colors: bool) -> bool {
        registry.names().count() != self.cached_object_count
            || registry.enabled_objects().count() != self.cached_enabled_count
            || registry.generation() != self.cached_generation
            || (viewer_colors && registry.has_any_dirty_molecule())
    }
}

// =============================================================================
// Pure functions (no egui dependency)
// =============================================================================

/// Compress a sorted list of residue numbers into range notation.
/// E.g., `[74, 75, 76, 77, 80, 85, 86, 87]` → `"74-77+80+85-87"`
pub fn compress_resi_list(values: &[i32]) -> String {
    if values.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    let mut start = values[0];
    let mut end = values[0];
    for &v in &values[1..] {
        if v == end + 1 {
            end = v;
        } else {
            if start == end {
                parts.push(start.to_string());
            } else {
                parts.push(format!("{}-{}", start, end));
            }
            start = v;
            end = v;
        }
    }
    if start == end {
        parts.push(start.to_string());
    } else {
        parts.push(format!("{}-{}", start, end));
    }
    parts.join("+")
}

/// Resolve an atom's named color index to RGB, if it has a named color.
fn resolve_named_color(atom: &Atom, named_palette: &NamedPalette) -> Option<[u8; 3]> {
    let color = atom.repr.colors.base;
    color.as_rgb8().or_else(|| {
        color
            .palette_index()
            .and_then(|id| named_palette.get_by_index(id).map(|rgb| rgb.to_rgb8()))
    })
}

/// Build sequence data for one molecule object.
///
/// Each `ChainView` covers all atoms sharing one chain identifier (polymer,
/// HET, solvent), so we get exactly one `SeqChain` per chain id. Per-residue
/// classification picks the display character: canonical amino acids and
/// nucleotides become single letters; ligands appear as `[HEM]`-style boxes;
/// waters and ions are skipped.
pub fn build_seq_object(
    object_name: &str,
    mol: &ObjectMolecule,
    color_ctx: &SequenceColorContext,
) -> SeqObject {
    let resolver =
        ColorResolver::new(color_ctx.named_palette, color_ctx.palette).with_molecule(mol);
    let defaults = RepColorDefaults::from_settings(color_ctx.settings);
    let mut chains: Vec<SeqChain> = Vec::new();

    for chain_view in mol.chains() {
        let mut residues: Vec<SeqResidue> = Vec::new();

        for residue_view in chain_view.residues() {
            let resn = residue_view.resn();

            let (display_label, kind) = if is_water(resn) || is_ion(resn) {
                continue;
            } else if is_capping_group(resn) {
                (format!("[{}]", resn), ResidueKind::AminoAcidNonCanonical)
            } else if residue_view.is_protein() {
                if is_standard_amino_acid(resn) {
                    let ch = three_to_one(resn).expect("standard AA must have 1-letter code");
                    (ch.to_string(), ResidueKind::AminoAcidCanonical)
                } else if let Some(ch) = three_to_one(resn) {
                    (ch.to_string(), ResidueKind::AminoAcidNonCanonical)
                } else {
                    (format!("[{}]", resn), ResidueKind::AminoAcidNonCanonical)
                }
            } else if residue_view.is_nucleic() {
                if is_standard_nucleotide(resn) {
                    let ch = nucleotide_to_char(resn)
                        .expect("standard nucleotide must have 1-letter code");
                    (ch.to_string(), ResidueKind::NucleotideCanonical)
                } else if let Some(ch) = nucleotide_to_char(resn) {
                    (ch.to_string(), ResidueKind::NucleotideNonCanonical)
                } else {
                    (format!("[{}]", resn), ResidueKind::NucleotideNonCanonical)
                }
            } else {
                (format!("[{}]", resn), ResidueKind::Ligand)
            };

            let display_char = if kind.is_polymer() {
                residue_to_char(resn)
            } else {
                '?'
            };
            let char_width = display_label.len();

            // Pick representative atom: CA for proteins, first atom otherwise
            let repr_atom = residue_view
                .ca()
                .map(|(_, atom)| atom)
                .or_else(|| residue_view.iter().next());

            let named_color = repr_atom
                .and_then(|a| resolve_named_color(a, color_ctx.named_palette))
                .unwrap_or([200, 200, 200]);

            let viewer_color = repr_atom
                .map(|a| {
                    let [r, g, b, _] = resolver.resolve_cartoon_with_defaults(a, &defaults);
                    Color::new(r, g, b).to_rgb8()
                })
                .unwrap_or([200, 200, 200]);

            residues.push(SeqResidue {
                chain: residue_view.chain().to_string(),
                resn: resn.to_string(),
                resv: residue_view.resv(),
                display_char,
                display_label,
                char_width,
                atom_range: residue_view.atom_range.clone(),
                named_color,
                viewer_color,
                kind,
            });
        }

        if !residues.is_empty() {
            chains.push(SeqChain {
                chain_id: chain_view.id().to_string(),
                residues,
            });
        }
    }

    SeqObject {
        object_name: object_name.to_string(),
        chains,
    }
}

/// Extract a one-letter-per-residue sequence string from a chain, skipping ligands.
pub fn chain_to_sequence(chain: &SeqChain) -> String {
    chain
        .residues
        .iter()
        .filter(|r| r.kind.is_polymer())
        .map(|r| {
            if r.display_char == '?' {
                'X'
            } else {
                r.display_char
            }
        })
        .collect()
}

/// Extract sequence of only the highlighted residues from a chain.
pub fn chain_highlighted_sequence(chain: &SeqChain, highlighted_resvs: &HashSet<i32>) -> String {
    chain
        .residues
        .iter()
        .filter(|r| r.kind.is_polymer())
        .filter(|r| highlighted_resvs.contains(&r.resv))
        .map(|r| {
            if r.display_char == '?' {
                'X'
            } else {
                r.display_char
            }
        })
        .collect()
}

/// Format `(header, sequence)` pairs as FASTA with 60-character line wrapping.
pub fn format_fasta(entries: &[(String, String)]) -> String {
    let mut result = String::new();
    for (header, seq) in entries {
        result.push('>');
        result.push_str(header);
        result.push('\n');
        for chunk in seq.as_bytes().chunks(60) {
            result.push_str(std::str::from_utf8(chunk).unwrap());
            result.push('\n');
        }
    }
    if result.ends_with('\n') {
        result.pop();
    }
    result
}

/// Format one polymer chain as FASTA.
pub fn chain_to_fasta(object_name: &str, chain: &SeqChain) -> Option<(String, usize)> {
    let sequence = chain_to_sequence(chain);
    if sequence.is_empty() {
        return None;
    }
    let header = format!("{}:{}", object_name, chain.chain_id);
    let count = sequence.len();
    Some((format_fasta(&[(header, sequence)]), count))
}

/// Collect all polymer sequences from all objects/chains.
/// Returns `(text, residue_count)` as FASTA.
pub fn collect_all_sequences(sequences: &[SeqObject]) -> (String, usize) {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut total = 0;
    for seq_obj in sequences {
        for chain in &seq_obj.chains {
            let seq = chain_to_sequence(chain);
            if !seq.is_empty() {
                total += seq.len();
                entries.push((format!("{}:{}", seq_obj.object_name, chain.chain_id), seq));
            }
        }
    }
    if entries.is_empty() {
        return (String::new(), 0);
    }
    (format_fasta(&entries), total)
}

#[cfg(test)]
mod tests {
    use lin_alg::f32::Vec3;
    use patinae_color::ColorValue;
    use patinae_color::{NamedPalette, ThemedPalette};
    use patinae_mol::{AtomBuilder, AtomFlags, AtomIndex, MoleculeBuilder};
    use patinae_scene::{DirtyFlags, MoleculeObject, ObjectRegistry};

    use super::*;

    fn test_registry() -> ObjectRegistry {
        let atom = AtomBuilder::new()
            .name("CA")
            .element_symbol("C")
            .resn("ALA")
            .resv(1)
            .chain("A")
            .flags(AtomFlags::PROTEIN | AtomFlags::POLYMER)
            .build();
        let mol = MoleculeBuilder::new("obj")
            .add_atom(atom, Vec3::new(0.0, 0.0, 0.0))
            .build();
        let mut registry = ObjectRegistry::new();
        registry.add(MoleculeObject::with_name(mol, "obj"));
        registry
    }

    fn rebuild_sequence_model(registry: &ObjectRegistry) -> SequenceModel {
        let named_palette = NamedPalette::new();
        let palette = ThemedPalette::dark();
        let color_ctx = SequenceColorContext {
            settings: &patinae_settings::Settings::default(),
            named_palette: &named_palette,
            palette: &palette,
        };
        let mut model = SequenceModel::new();
        model.rebuild_cache(registry, &color_ctx);
        model
    }

    fn residue(label: &str, display_char: char, kind: ResidueKind, resv: i32) -> SeqResidue {
        SeqResidue {
            chain: "A".to_string(),
            resn: label.to_string(),
            resv,
            display_char,
            display_label: display_char.to_string(),
            char_width: 1,
            atom_range: 0..1,
            named_color: [0, 0, 0],
            viewer_color: [0, 0, 0],
            kind,
        }
    }

    fn chain(id: &str, residues: Vec<SeqResidue>) -> SeqChain {
        SeqChain {
            chain_id: id.to_string(),
            residues,
        }
    }

    #[test]
    fn viewer_colors_rebuild_when_molecule_is_dirty() {
        let mut registry = test_registry();
        let model = rebuild_sequence_model(&registry);
        registry.clear_all_dirty_molecules();

        registry
            .get_molecule_mut("obj")
            .unwrap()
            .invalidate(DirtyFlags::COLOR);

        assert!(model.needs_rebuild(&registry, true));
    }

    #[test]
    fn viewer_colors_do_not_scan_clean_atom_colors() {
        let mut registry = test_registry();
        let model = rebuild_sequence_model(&registry);
        registry.clear_all_dirty_molecules();

        {
            let mol_obj = registry.get_molecule_mut("obj").unwrap();
            let mol = mol_obj.molecule_mut();
            let atom = mol.get_atom_mut(AtomIndex(0)).unwrap();
            atom.repr.colors.base = ColorValue::palette(42);
            atom.repr.colors.cartoon = ColorValue::palette(42);
        }
        registry.clear_all_dirty_molecules();

        assert!(!model.needs_rebuild(&registry, true));
    }

    #[test]
    fn chain_to_fasta_formats_single_chain_with_header() {
        let chain = chain(
            "A",
            vec![
                residue("ALA", 'A', ResidueKind::AminoAcidCanonical, 1),
                residue("UNK", '?', ResidueKind::AminoAcidNonCanonical, 2),
                residue("HEM", '?', ResidueKind::Ligand, 3),
            ],
        );

        let (text, count) = chain_to_fasta("obj", &chain).unwrap();

        assert_eq!(count, 2);
        assert_eq!(text, ">obj:A\nAX");
    }

    #[test]
    fn collect_all_sequences_returns_fasta_even_for_one_chain() {
        let sequences = vec![SeqObject {
            object_name: "obj".to_string(),
            chains: vec![chain(
                "A",
                vec![residue("ALA", 'A', ResidueKind::AminoAcidCanonical, 1)],
            )],
        }];

        let (text, count) = collect_all_sequences(&sequences);

        assert_eq!(count, 1);
        assert_eq!(text, ">obj:A\nA");
    }

    #[test]
    fn format_fasta_wraps_long_sequences() {
        let sequence = "A".repeat(61);
        let text = format_fasta(&[("obj:A".to_string(), sequence)]);

        assert_eq!(text, format!(">obj:A\n{}\nA", "A".repeat(60)));
    }
}
