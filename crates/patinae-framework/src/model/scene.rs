//! Scene Model
//!
//! UI-framework-agnostic representation of the molecular scene for the objects
//! panel. The hierarchy is Group → Object → Subchain, where each *subchain* is a
//! homogeneous polymer / HET / solvent run within a chain. The `SubchainKind`
//! drives sidebar color, popover commands, and selection scope — see
//! [`patinae_mol::SubchainKind`].

use std::hash::{DefaultHasher, Hash, Hasher};

use patinae_color::{Color, ColorIndex, NamedPalette, ThemedPalette};
use patinae_mol::{Atom, SubchainKind, SubchainView};
use patinae_scene::{
    resolve_annotation_bundles, AnnotationColorSummary, MapDisplayMode, MeasurementKind, Object,
    ObjectRegistry, ResolvedAnnotationBundle,
};
use patinae_select::format_exact_selector_value;
use patinae_settings::Settings;

use crate::model::sequence::compress_resi_list;

// ============================================================================
// Sidebar color
// ============================================================================

/// Semantic sidebar dot color for a subchain.
///
/// The UI layer maps these to concrete colors (theme-dependent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SidebarColor {
    /// Water / solvent — UI picks the color (e.g. blue).
    Solvent,
    /// Non-biopolymer subchain — UI picks the color (e.g. gray).
    Other,
    /// Multi-color scheme (by_ss, by_residue, by_index) — no single representative.
    Multicolor,
    /// A specific resolved color.
    Color(Color),
}

// ============================================================================
// Domain model
// ============================================================================

/// One subchain row in the Objects panel — a polymer stretch, a single ligand
/// residue (HEM), a run of waters, or an ion group.
#[derive(Debug, Clone)]
pub struct SceneSubchain {
    /// Raw chain identifier (for example "A") used for command generation.
    pub chain_id: String,
    /// Subchain classification.
    pub kind: SubchainKind,
    /// Display label: residue name (e.g. "HEM") for non-polymer subchains,
    /// chain identifier for biopolymer subchains. Display only — never use
    /// this in selection commands (composite glycans render as `"NAG+8"`).
    pub label: String,
    /// Atom count in this subchain.
    pub atom_count: usize,
    /// Representative sidebar dot color.
    pub color: SidebarColor,
    /// Index into the owning molecule's `SubchainPartition::entries()`.
    pub entry_index: u32,
    /// Selection clause appended after `"{obj_name} and "` to scope a
    /// command to this subchain. Empty when the row covers the whole object
    /// (single subchain in object) — bridge then emits just `obj_name`.
    /// Built from typed `SubchainKind`/`SubchainLabel`, never from `label`.
    pub selector_clause: String,
}

impl SceneSubchain {
    /// Display suffix shown next to the chain identifier in the sidebar.
    /// Empty when the label is the chain identifier itself (biopolymer rows
    /// that don't need disambiguation).
    pub fn display_label(&self) -> &str {
        if self.label == self.chain_id {
            ""
        } else {
            &self.label
        }
    }
}

/// A loaded molecular object.
#[derive(Debug, Clone)]
pub struct SceneObject {
    pub name: String,
    pub kind: SceneObjectKind,
    pub map_visual_kind: Option<SceneMapVisualKind>,
    /// Measurement subtype used for iconography and row metadata.
    pub measurement_kind: Option<MeasurementKind>,
    /// Number of stored entities for measurement and label collections.
    pub entity_count: usize,
    /// Whether any annotation entity cannot currently be resolved.
    pub has_unresolved_entities: bool,
    /// Human-readable reason focus is unavailable for this object.
    pub focus_disabled_reason: Option<String>,
    /// Actions that are valid for this object kind.
    pub capabilities: SceneObjectCapabilities,
    /// Resolved sidebar color for the object row.
    pub color: SidebarColor,
    pub enabled: bool,
    /// Whether subchains are shown in the sidebar.
    pub expanded: bool,
    pub subchains: Vec<SceneSubchain>,
}

/// Semantic object kind used by the Objects panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneObjectKind {
    Molecule,
    Map,
    Measurement,
    Label,
}

/// Actions supported by one object-list row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceneObjectCapabilities {
    pub focus: bool,
    pub visibility: bool,
    pub color: bool,
    pub rename: bool,
    pub delete: bool,
    pub grouping: bool,
    pub representations: bool,
    pub copy: bool,
    pub extract: bool,
    pub align: bool,
    pub orient: bool,
    pub remove_atoms: bool,
}

impl SceneObjectCapabilities {
    const fn molecule() -> Self {
        Self {
            focus: true,
            visibility: true,
            color: true,
            rename: true,
            delete: true,
            grouping: true,
            representations: true,
            copy: true,
            extract: true,
            align: true,
            orient: true,
            remove_atoms: true,
        }
    }

    const fn map() -> Self {
        Self {
            focus: true,
            visibility: true,
            color: true,
            rename: true,
            delete: true,
            grouping: true,
            representations: false,
            copy: true,
            extract: true,
            align: true,
            orient: true,
            remove_atoms: true,
        }
    }

    const fn annotation(has_extent: bool) -> Self {
        Self {
            focus: has_extent,
            visibility: true,
            color: true,
            rename: true,
            delete: true,
            grouping: true,
            representations: false,
            copy: false,
            extract: false,
            align: false,
            orient: false,
            remove_atoms: false,
        }
    }
}

/// Visual subtype used by map rows in the Objects panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneMapVisualKind {
    Source,
    Isomesh,
    Isosurface,
}

/// A group of objects.
#[derive(Debug, Clone)]
pub struct SceneGroup {
    pub name: String,
    /// Whether the group's children are visible in the sidebar.
    pub open: bool,
    pub enabled: bool,
    /// Child objects in render order.
    pub children: Vec<SceneObject>,
}

/// A top-level entry in the scene: either a group or a standalone object.
#[derive(Debug, Clone)]
pub enum SceneEntry {
    Group(SceneGroup),
    Object(SceneObject),
}

/// Everything needed to resolve sidebar colors.
pub struct SceneColorContext<'a> {
    pub named_palette: &'a NamedPalette,
    pub palette: &'a ThemedPalette,
    pub settings: &'a Settings,
}

/// UI-framework-agnostic scene model for the objects panel.
pub struct SceneModel {
    pub entries: Vec<SceneEntry>,
    last_generation: u64,
    last_annotation_settings_hash: u64,
}

impl SceneModel {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            last_generation: u64::MAX, // force first sync
            last_annotation_settings_hash: u64::MAX,
        }
    }

    /// Sync from the registry. Returns `true` if the model was rebuilt.
    ///
    /// Triggers a rebuild when either:
    /// - The registry generation changed (structural: add/remove/enable/disable)
    /// - Any molecule has pending dirty flags (content: color/coords/reps)
    pub fn sync(&mut self, registry: &ObjectRegistry, color_ctx: &SceneColorContext<'_>) -> bool {
        let gen = registry.generation();
        let annotation_settings_hash = annotation_settings_hash(color_ctx.settings);
        if gen == self.last_generation
            && annotation_settings_hash == self.last_annotation_settings_hash
            && !registry.has_any_dirty_molecule()
            && !registry.has_any_dirty_map()
            && !registry.has_any_dirty_measurement()
            && !registry.has_any_dirty_label()
        {
            return false;
        }
        self.last_generation = gen;
        self.last_annotation_settings_hash = annotation_settings_hash;
        self.rebuild(registry, color_ctx);
        true
    }

    /// Force a rebuild on the next `sync` call.
    pub fn invalidate(&mut self) {
        self.last_generation = u64::MAX;
        self.last_annotation_settings_hash = u64::MAX;
    }

    /// Toggle expand/collapse for an object (searches both standalone and group children).
    pub fn toggle_expanded(&mut self, name: &str) {
        for entry in &mut self.entries {
            match entry {
                SceneEntry::Object(obj) if obj.name == name => {
                    obj.expanded = !obj.expanded;
                    return;
                }
                SceneEntry::Group(group) => {
                    if let Some(obj) = group.children.iter_mut().find(|o| o.name == name) {
                        obj.expanded = !obj.expanded;
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// Toggle open/close for a group.
    pub fn toggle_group_open(&mut self, name: &str) {
        for entry in &mut self.entries {
            if let SceneEntry::Group(group) = entry {
                if group.name == name {
                    group.open = !group.open;
                    return;
                }
            }
        }
    }

    /// Lookup a standalone object or group child by name.
    pub fn get(&self, name: &str) -> Option<&SceneObject> {
        for entry in &self.entries {
            match entry {
                SceneEntry::Object(obj) if obj.name == name => return Some(obj),
                SceneEntry::Group(group) => {
                    if let Some(obj) = group.children.iter().find(|o| o.name == name) {
                        return Some(obj);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Lookup a group by name.
    pub fn get_group(&self, name: &str) -> Option<&SceneGroup> {
        for entry in &self.entries {
            if let SceneEntry::Group(group) = entry {
                if group.name == name {
                    return Some(group);
                }
            }
        }
        None
    }

    fn rebuild(&mut self, registry: &ObjectRegistry, color_ctx: &SceneColorContext<'_>) {
        // Preserve expanded/open state across rebuilds
        let was_expanded: std::collections::HashSet<String> = self
            .iter_objects()
            .filter(|o| o.expanded)
            .map(|o| o.name.clone())
            .collect();
        let was_open: std::collections::HashSet<String> = self
            .entries
            .iter()
            .filter_map(|e| match e {
                SceneEntry::Group(g) if g.open => Some(g.name.clone()),
                _ => None,
            })
            .collect();

        self.entries.clear();
        let annotations =
            resolve_annotation_bundles(registry, color_ctx.settings, color_ctx.named_palette);
        let annotations_by_owner = annotations
            .iter()
            .map(|bundle| (bundle.owner_name.as_str(), bundle))
            .collect::<std::collections::HashMap<_, _>>();

        for top_name in registry.top_level_names() {
            if let Some(group) = registry.get_group(top_name) {
                // Build group with its children
                let children: Vec<SceneObject> = group
                    .children()
                    .iter()
                    .filter_map(|child_name| {
                        build_object_by_name(
                            registry,
                            child_name,
                            was_expanded.contains(child_name.as_str()),
                            color_ctx,
                            &annotations_by_owner,
                        )
                    })
                    .collect();

                self.entries.push(SceneEntry::Group(SceneGroup {
                    name: top_name.to_string(),
                    open: was_open.contains(top_name),
                    enabled: group.state().enabled,
                    children,
                }));
            } else if let Some(object) = build_object_by_name(
                registry,
                top_name,
                was_expanded.contains(top_name),
                color_ctx,
                &annotations_by_owner,
            ) {
                self.entries.push(SceneEntry::Object(object));
            }
        }
    }

    /// Iterate over all objects (both standalone and inside groups).
    fn iter_objects(&self) -> impl Iterator<Item = &SceneObject> {
        self.entries.iter().flat_map(|entry| match entry {
            SceneEntry::Object(obj) => std::slice::from_ref(obj).iter().chain([].iter()),
            SceneEntry::Group(group) => [].iter().chain(group.children.iter()),
        })
    }
}

impl Default for SceneModel {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn build_object_by_name(
    registry: &ObjectRegistry,
    name: &str,
    expanded: bool,
    color_ctx: &SceneColorContext<'_>,
    annotations: &std::collections::HashMap<&str, &ResolvedAnnotationBundle>,
) -> Option<SceneObject> {
    if let Some(molecule) = registry.get_molecule(name) {
        Some(build_scene_object(name, molecule, expanded, color_ctx))
    } else if let Some(map) = registry.get_map(name) {
        Some(build_map_scene_object(name, map, expanded))
    } else {
        registry
            .get_measurement(name)
            .map(|measurement| {
                build_measurement_scene_object(
                    name,
                    measurement,
                    annotations.get(name).copied(),
                    color_ctx,
                )
            })
            .or_else(|| {
                registry.get_label(name).map(|label| {
                    build_label_scene_object(name, label, annotations.get(name).copied(), color_ctx)
                })
            })
    }
}

fn build_scene_object(
    name: &str,
    mol: &patinae_scene::MoleculeObject,
    expanded: bool,
    color_ctx: &SceneColorContext<'_>,
) -> SceneObject {
    let state = mol.state();
    let molecule = mol.molecule();
    let partition = molecule.subchain_partition();
    let atoms = molecule.atoms_slice();
    let total_entries = partition.entries().len();

    // Count entries per chain so we can decide between Case A (single
    // subchain in chain → `chain X`) and Case B (multiple subchains →
    // chain X + kind-specific qualifier).
    let mut entries_per_chain: std::collections::HashMap<&str, u32> =
        std::collections::HashMap::new();
    for e in partition.entries() {
        *entries_per_chain.entry(e.chain_id.as_str()).or_insert(0) += 1;
    }

    let subchains: Vec<SceneSubchain> = partition
        .entries()
        .iter()
        .enumerate()
        .map(|(idx, _)| {
            let view = partition
                .view_for(idx as u32, atoms)
                .expect("entry index in range");
            let chain_id = view.chain_id().to_string();
            let label = view.label().to_string();
            let color = resolve_sidebar_color(&view, color_ctx);
            let chain_count = entries_per_chain
                .get(chain_id.as_str())
                .copied()
                .unwrap_or(1);
            let selector_clause = build_selector_clause(&view, total_entries, chain_count);
            SceneSubchain {
                chain_id,
                kind: view.kind,
                label,
                atom_count: view.len(),
                color,
                entry_index: idx as u32,
                selector_clause,
            }
        })
        .collect();

    let color = subchains
        .first()
        .map(|subchain| subchain.color)
        .unwrap_or(SidebarColor::Other);

    SceneObject {
        name: name.to_string(),
        kind: SceneObjectKind::Molecule,
        map_visual_kind: None,
        measurement_kind: None,
        entity_count: 0,
        has_unresolved_entities: false,
        focus_disabled_reason: None,
        capabilities: SceneObjectCapabilities::molecule(),
        color,
        enabled: state.enabled,
        expanded,
        subchains,
    }
}

fn build_map_scene_object(
    name: &str,
    map: &patinae_scene::MapObject,
    expanded: bool,
) -> SceneObject {
    SceneObject {
        name: name.to_string(),
        kind: SceneObjectKind::Map,
        map_visual_kind: Some(map_visual_kind(map.display_mode())),
        measurement_kind: None,
        entity_count: 0,
        has_unresolved_entities: false,
        focus_disabled_reason: None,
        capabilities: SceneObjectCapabilities::map(),
        color: SidebarColor::Color(Color::WHITE),
        enabled: map.state().enabled,
        expanded,
        subchains: Vec::new(),
    }
}

fn build_measurement_scene_object(
    name: &str,
    measurement: &patinae_scene::MeasurementObject,
    bundle: Option<&ResolvedAnnotationBundle>,
    color_ctx: &SceneColorContext<'_>,
) -> SceneObject {
    let has_extent = bundle.is_some_and(|bundle| bundle.bounds.is_some());
    let fallback_color = if measurement.has_explicit_color() {
        resolve_object_color(measurement.state().color, color_ctx.named_palette)
    } else {
        SidebarColor::Color(patinae_color::CYAN)
    };
    SceneObject {
        name: name.to_string(),
        kind: SceneObjectKind::Measurement,
        map_visual_kind: None,
        measurement_kind: Some(measurement.kind()),
        entity_count: measurement.len(),
        has_unresolved_entities: bundle.is_some_and(|bundle| bundle.unresolved_count != 0),
        focus_disabled_reason: (!has_extent).then(|| "No resolvable anchors".to_string()),
        capabilities: SceneObjectCapabilities::annotation(has_extent),
        color: annotation_sidebar_color(bundle, fallback_color),
        enabled: measurement.state().enabled,
        expanded: false,
        subchains: Vec::new(),
    }
}

fn build_label_scene_object(
    name: &str,
    label: &patinae_scene::LabelObject,
    bundle: Option<&ResolvedAnnotationBundle>,
    color_ctx: &SceneColorContext<'_>,
) -> SceneObject {
    let has_extent = bundle.is_some_and(|bundle| bundle.bounds.is_some());
    let fallback_color = label
        .presentation()
        .color()
        .map_or(SidebarColor::Color(patinae_color::CYAN), |color| {
            resolve_object_color(color, color_ctx.named_palette)
        });
    SceneObject {
        name: name.to_string(),
        kind: SceneObjectKind::Label,
        map_visual_kind: None,
        measurement_kind: None,
        entity_count: label.len(),
        has_unresolved_entities: bundle.is_some_and(|bundle| bundle.unresolved_count != 0),
        focus_disabled_reason: (!has_extent).then(|| "No resolvable anchors".to_string()),
        capabilities: SceneObjectCapabilities::annotation(has_extent),
        color: annotation_sidebar_color(bundle, fallback_color),
        enabled: label.state().enabled,
        expanded: false,
        subchains: Vec::new(),
    }
}

fn annotation_sidebar_color(
    bundle: Option<&ResolvedAnnotationBundle>,
    fallback: SidebarColor,
) -> SidebarColor {
    match bundle.map(ResolvedAnnotationBundle::color_summary) {
        Some(AnnotationColorSummary::Uniform([red, green, blue])) => {
            SidebarColor::Color(Color::new(red, green, blue))
        }
        Some(AnnotationColorSummary::Mixed) => SidebarColor::Multicolor,
        Some(AnnotationColorSummary::Empty) | None => fallback,
    }
}

fn resolve_object_color(color: ColorIndex, named_palette: &NamedPalette) -> SidebarColor {
    match color {
        ColorIndex::Named(index) => SidebarColor::Color(
            named_palette
                .get_by_index(index)
                .unwrap_or(patinae_color::CYAN),
        ),
        _ => SidebarColor::Color(patinae_color::CYAN),
    }
}

fn annotation_settings_hash(settings: &Settings) -> u64 {
    let measurement = &settings.measurement;
    let mut hasher = DefaultHasher::new();
    for value in [
        measurement.dash_length,
        measurement.dash_gap,
        measurement.dash_width,
        measurement.angle_size,
        measurement.angle_label_position,
        measurement.dihedral_size,
        measurement.dihedral_label_position,
        measurement.dash_transparency,
        measurement.label_size,
    ] {
        value.to_bits().hash(&mut hasher);
    }
    measurement.dash_round_ends.hash(&mut hasher);
    for value in [
        measurement.label_digits,
        measurement.label_distance_digits,
        measurement.label_angle_digits,
        measurement.label_dihedral_digits,
        measurement.dash_color.0,
        measurement.angle_color.0,
        measurement.dihedral_color.0,
    ] {
        value.hash(&mut hasher);
    }
    hasher.finish()
}

fn map_visual_kind(mode: MapDisplayMode) -> SceneMapVisualKind {
    match mode {
        MapDisplayMode::Isomesh => SceneMapVisualKind::Isomesh,
        MapDisplayMode::Isosurface => SceneMapVisualKind::Isosurface,
        MapDisplayMode::None | MapDisplayMode::Isodot | MapDisplayMode::Volume => {
            SceneMapVisualKind::Source
        }
    }
}

/// Synthesize the selection clause appended to `"{obj_name} and "`.
///
/// - Whole-object row (single subchain in object) → `""`.
/// - Single subchain in its chain → `"chain X"` (no kind qualifier).
/// - Multiple subchains in a chain → `"chain X and {qualifier}"` chosen
///   from `SubchainKind`:
///   - `Biopolymer`        → `polymer`
///   - `Solvent`           → `solvent`
///   - `Inorganic`         → `resn A+B+…`
///   - `Organic` / `Lipid` → `organic and resi 1-3+5+7-9`
///   - `Other`             → `resn A+B+…`
fn build_selector_clause(sub: &SubchainView<'_>, total_entries: usize, chain_count: u32) -> String {
    if total_entries <= 1 {
        return String::new();
    }
    let chain_id = format_exact_selector_value(sub.chain_id());
    let chain_clause = format!("chain {chain_id}");
    if chain_count <= 1 {
        return chain_clause;
    }
    match sub.kind {
        SubchainKind::Biopolymer => format!("{} and polymer", chain_clause),
        SubchainKind::Solvent => format!("{} and solvent", chain_clause),
        SubchainKind::Inorganic | SubchainKind::Other => {
            let resns = collect_distinct_resns(sub);
            if resns.is_empty() {
                chain_clause
            } else {
                format!("{} and resn {}", chain_clause, resns.join("+"))
            }
        }
        SubchainKind::Organic => {
            let resi = compress_resvs(sub);
            if resi.is_empty() {
                format!("{} and organic", chain_clause)
            } else {
                format!("{} and organic and resi {}", chain_clause, resi)
            }
        }
    }
}

fn collect_distinct_resns(sub: &SubchainView<'_>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for atom in sub.iter() {
        let resn: &str = atom.residue.resn.as_str();
        if resn.is_empty() {
            continue;
        }
        if !out.iter().any(|r| r.as_str() == resn) {
            out.push(resn.to_string());
        }
    }
    out
}

fn compress_resvs(sub: &SubchainView<'_>) -> String {
    let mut values: Vec<i32> = sub.iter().map(|a| a.residue.resv).collect();
    values.sort_unstable();
    values.dedup();
    compress_resi_list(&values)
}

// ============================================================================
// Sidebar color resolution
// ============================================================================

/// Determine the representative color for a subchain's sidebar dot.
fn resolve_sidebar_color(sub: &SubchainView<'_>, ctx: &SceneColorContext<'_>) -> SidebarColor {
    match sub.kind {
        SubchainKind::Solvent => SidebarColor::Solvent,
        SubchainKind::Biopolymer => resolve_biopolymer_color(sub, ctx),
        _ => SidebarColor::Other,
    }
}

/// For biopolymers: use effective cartoon color (cartoon_or_base) so the
/// sidebar dot matches the cartoon representation in the viewport.
/// ByChain → palette color, multicolor schemes → Multicolor,
/// otherwise → consensus CA cartoon color.
fn resolve_biopolymer_color(sub: &SubchainView<'_>, ctx: &SceneColorContext<'_>) -> SidebarColor {
    let first_carbon = sub.iter().find(|a| a.is_ca());
    let Some(fc) = first_carbon else {
        return SidebarColor::Other;
    };

    match ColorIndex::from(fc.repr.colors.cartoon_or_base()) {
        ColorIndex::ByChain => SidebarColor::Color(ctx.palette.chains.get(sub.chain_id())),
        ColorIndex::BySS | ColorIndex::ByResidueType | ColorIndex::ByResidueIndex => {
            SidebarColor::Multicolor
        }
        _ => {
            // Named color or ByElement — check if all CAs share one color
            uniform_ca_color(sub, ctx)
        }
    }
}

/// If all CA atoms in the subchain resolve to the same color, return it.
/// Otherwise return Multicolor. Returns Other if no CAs found.
fn uniform_ca_color(sub: &SubchainView<'_>, ctx: &SceneColorContext<'_>) -> SidebarColor {
    let mut first: Option<[u8; 3]> = None;
    let mut first_color = Color::WHITE;
    for atom in sub.iter() {
        if !atom.is_ca() {
            continue;
        }
        let color = resolve_atom_color(atom, ctx);
        let key = [
            (color.r * 255.0) as u8,
            (color.g * 255.0) as u8,
            (color.b * 255.0) as u8,
        ];
        match first {
            None => {
                first = Some(key);
                first_color = color;
            }
            Some(k) if k != key => return SidebarColor::Multicolor,
            _ => {}
        }
    }
    first
        .map(|_| SidebarColor::Color(first_color))
        .unwrap_or(SidebarColor::Other)
}

/// Resolve a single atom's effective cartoon color for sidebar purposes.
fn resolve_atom_color(atom: &Atom, ctx: &SceneColorContext<'_>) -> Color {
    match ColorIndex::from(atom.repr.colors.cartoon_or_base()) {
        ColorIndex::ByElement | ColorIndex::Atomic => ctx.palette.element.get(atom.element as u8),
        ColorIndex::ByChain => {
            if atom.state.flags.is_biomolecule() && atom.element.is_carbon() {
                ctx.palette.chains.get(&atom.residue.chain)
            } else {
                ctx.palette.element.get(atom.element as u8)
            }
        }
        ColorIndex::Named(idx) => ctx.named_palette.get_by_index(idx).unwrap_or(Color::WHITE),
        _ => Color::new(0.5, 0.5, 0.5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lin_alg::f32::Vec3;
    use patinae_algos::surface::Grid3D;
    use patinae_mol::{
        Atom, AtomBuilder, AtomFlags, AtomIndex, Element, MoleculeBuilder, ObjectMolecule,
    };
    use patinae_scene::{
        AtomAnchor, LabelEntity, LabelEntityPresentation, LabelObject, MapObject,
        MeasurementAnchor, MeasurementEntry, MeasurementObject, MoleculeObject,
    };

    fn poly_atom(chain: &str, resn: &str, resv: i32, name: &str) -> Atom {
        AtomBuilder::new()
            .name(name)
            .element(Element::Carbon)
            .chain(chain)
            .resn(resn)
            .resv(resv)
            .flags(AtomFlags::PROTEIN)
            .build()
    }

    fn organic_atom(chain: &str, resn: &str, resv: i32, name: &str) -> Atom {
        AtomBuilder::new()
            .name(name)
            .element(Element::Carbon)
            .chain(chain)
            .resn(resn)
            .resv(resv)
            .hetatm(true)
            .flags(AtomFlags::ORGANIC)
            .build()
    }

    fn solvent_atom(chain: &str, resv: i32) -> Atom {
        AtomBuilder::new()
            .name("O")
            .element(Element::Oxygen)
            .chain(chain)
            .resn("HOH")
            .resv(resv)
            .hetatm(true)
            .flags(AtomFlags::SOLVENT)
            .build()
    }

    fn inorganic_atom(chain: &str, resn: &str, resv: i32) -> Atom {
        AtomBuilder::new()
            .name(resn)
            .element(Element::Carbon)
            .chain(chain)
            .resn(resn)
            .resv(resv)
            .hetatm(true)
            .flags(AtomFlags::INORGANIC)
            .build()
    }

    /// Build a `SubchainView` for the entry at `idx` and call
    /// `build_selector_clause` against it.
    fn clause_for(mol: &ObjectMolecule, idx: u32) -> String {
        let partition = mol.subchain_partition();
        let total = partition.entries().len();
        let view = partition
            .view_for(idx, mol.atoms_slice())
            .expect("entry exists");
        let chain_count = partition
            .entries()
            .iter()
            .filter(|e| e.chain_id == view.chain_id())
            .count() as u32;
        build_selector_clause(&view, total, chain_count)
    }

    #[test]
    fn subchain_kind_as_str_roundtrip() {
        assert_eq!(SubchainKind::Biopolymer.as_str(), "biopolymer");
        assert_eq!(SubchainKind::Solvent.as_str(), "solvent");
        assert_eq!(SubchainKind::Organic.as_str(), "organic");
        assert_eq!(SubchainKind::Inorganic.as_str(), "inorganic");
        assert_eq!(SubchainKind::Other.as_str(), "other");
    }

    #[test]
    fn scene_subchain_display_label_hides_redundant_chain_id() {
        let s = SceneSubchain {
            chain_id: "A".to_string(),
            kind: SubchainKind::Biopolymer,
            label: "A".to_string(),
            atom_count: 100,
            color: SidebarColor::Other,
            entry_index: 0,
            selector_clause: String::new(),
        };
        assert_eq!(s.display_label(), "");
    }

    #[test]
    fn scene_subchain_display_label_shows_resn_for_het() {
        let s = SceneSubchain {
            chain_id: "A".to_string(),
            kind: SubchainKind::Organic,
            label: "HEM".to_string(),
            atom_count: 4,
            color: SidebarColor::Other,
            entry_index: 0,
            selector_clause: String::new(),
        };
        assert_eq!(s.display_label(), "HEM");
    }

    #[test]
    fn scene_model_starts_empty() {
        let model = SceneModel::new();
        assert!(model.entries.is_empty());
    }

    #[test]
    fn scene_model_no_change_returns_false() {
        let mut model = SceneModel::new();
        let registry = ObjectRegistry::default();
        let palette = ThemedPalette::dark();
        let named = NamedPalette::default();
        let mut settings = Settings::default();
        let ctx = SceneColorContext {
            named_palette: &named,
            palette: &palette,
            settings: &settings,
        };
        // First sync rebuilds
        assert!(model.sync(&registry, &ctx));
        // Second sync with no changes
        assert!(!model.sync(&registry, &ctx));

        settings.measurement.label_size = 18.0;
        let changed_ctx = SceneColorContext {
            named_palette: &named,
            palette: &palette,
            settings: &settings,
        };
        assert!(model.sync(&registry, &changed_ctx));
    }

    #[test]
    fn invalidate_forces_rebuild() {
        let mut model = SceneModel::new();
        let registry = ObjectRegistry::default();
        let palette = ThemedPalette::dark();
        let ctx = SceneColorContext {
            named_palette: &NamedPalette::default(),
            palette: &palette,
            settings: &Settings::default(),
        };
        model.sync(&registry, &ctx);
        model.invalidate();
        assert!(model.sync(&registry, &ctx));
    }

    #[test]
    fn sidebar_color_variants() {
        assert_eq!(SidebarColor::Solvent, SidebarColor::Solvent);
        assert_eq!(SidebarColor::Multicolor, SidebarColor::Multicolor);
        assert_eq!(SidebarColor::Other, SidebarColor::Other);
        let c = SidebarColor::Color(Color::WHITE);
        assert!(matches!(c, SidebarColor::Color(_)));
    }

    #[test]
    fn toggle_group_open() {
        let mut model = SceneModel::new();
        model.entries.push(SceneEntry::Group(SceneGroup {
            name: "grp".to_string(),
            open: false,
            enabled: true,
            children: vec![],
        }));
        assert!(!model.get_group("grp").unwrap().open);
        model.toggle_group_open("grp");
        assert!(model.get_group("grp").unwrap().open);
        model.toggle_group_open("grp");
        assert!(!model.get_group("grp").unwrap().open);
    }

    #[test]
    fn get_finds_objects_inside_groups() {
        let mut model = SceneModel::new();
        let obj = SceneObject {
            name: "mol".to_string(),
            kind: SceneObjectKind::Molecule,
            map_visual_kind: None,
            measurement_kind: None,
            entity_count: 0,
            has_unresolved_entities: false,
            focus_disabled_reason: None,
            capabilities: SceneObjectCapabilities::molecule(),
            color: SidebarColor::Other,
            enabled: true,
            expanded: false,
            subchains: vec![],
        };
        model.entries.push(SceneEntry::Group(SceneGroup {
            name: "grp".to_string(),
            open: true,
            enabled: true,
            children: vec![obj],
        }));
        assert!(model.get("mol").is_some());
        assert_eq!(model.get("mol").unwrap().name, "mol");
    }

    #[test]
    fn toggle_expanded_inside_group() {
        let mut model = SceneModel::new();
        let obj = SceneObject {
            name: "mol".to_string(),
            kind: SceneObjectKind::Molecule,
            map_visual_kind: None,
            measurement_kind: None,
            entity_count: 0,
            has_unresolved_entities: false,
            focus_disabled_reason: None,
            capabilities: SceneObjectCapabilities::molecule(),
            color: SidebarColor::Other,
            enabled: true,
            expanded: false,
            subchains: vec![],
        };
        model.entries.push(SceneEntry::Group(SceneGroup {
            name: "grp".to_string(),
            open: true,
            enabled: true,
            children: vec![obj],
        }));
        assert!(!model.get("mol").unwrap().expanded);
        model.toggle_expanded("mol");
        assert!(model.get("mol").unwrap().expanded);
    }

    #[test]
    fn scene_model_maps_report_visual_kind_by_display_mode() {
        let mut registry = ObjectRegistry::default();
        let grid = Grid3D::from_dims([0.0; 3], [1.0; 3], [1, 1, 1], vec![0.0; 8]);
        registry.add(MapObject::new("source", grid.clone()));

        let mut mesh = MapObject::new("mesh", grid.clone());
        mesh.set_display_mode(MapDisplayMode::Isomesh);
        registry.add(mesh);

        let mut surface = MapObject::new("surface", grid);
        surface.set_display_mode(MapDisplayMode::Isosurface);
        registry.add(surface);

        let mut model = SceneModel::new();
        let palette = ThemedPalette::dark();
        let ctx = SceneColorContext {
            named_palette: &NamedPalette::default(),
            palette: &palette,
            settings: &Settings::default(),
        };
        assert!(model.sync(&registry, &ctx));

        assert_eq!(
            model.get("source").and_then(|obj| obj.map_visual_kind),
            Some(SceneMapVisualKind::Source)
        );
        assert_eq!(
            model.get("mesh").and_then(|obj| obj.map_visual_kind),
            Some(SceneMapVisualKind::Isomesh)
        );
        assert_eq!(
            model.get("surface").and_then(|obj| obj.map_visual_kind),
            Some(SceneMapVisualKind::Isosurface)
        );
    }

    #[test]
    fn scene_model_includes_measurements_with_kind_count_and_capabilities() {
        let molecule = MoleculeBuilder::new("source")
            .add_atom(poly_atom("A", "ALA", 1, "CA"), Vec3::new(0.0, 0.0, 0.0))
            .add_atom(poly_atom("A", "ALA", 2, "CA"), Vec3::new(2.0, 0.0, 0.0))
            .build();
        let first = AtomIndex(0);
        let second = AtomIndex(1);

        let mut registry = ObjectRegistry::default();
        registry.add(MoleculeObject::new(molecule));
        let mut measurement = MeasurementObject::new("d1", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntry::new(vec![
                MeasurementAnchor::new("source", first),
                MeasurementAnchor::new("source", second),
            ]))
            .expect("distance entry");
        registry.add(measurement);

        let mut model = SceneModel::new();
        let palette = ThemedPalette::dark();
        let ctx = SceneColorContext {
            named_palette: &NamedPalette::default(),
            palette: &palette,
            settings: &Settings::default(),
        };
        assert!(model.sync(&registry, &ctx));

        let object = model.get("d1").expect("measurement row");
        assert_eq!(object.kind, SceneObjectKind::Measurement);
        assert_eq!(object.measurement_kind, Some(MeasurementKind::Distance));
        assert_eq!(object.entity_count, 1);
        assert!(!object.has_unresolved_entities);
        assert_eq!(object.color, SidebarColor::Color(patinae_color::CYAN));
        assert!(object.capabilities.focus);
        assert!(object.capabilities.color);
        assert!(object.capabilities.delete);
        assert!(object.capabilities.grouping);
        assert!(!object.capabilities.representations);
        assert!(!object.capabilities.copy);
        assert!(!object.capabilities.extract);
        assert!(!object.capabilities.align);
        assert!(!object.capabilities.orient);
        assert!(!object.capabilities.remove_atoms);
    }

    #[test]
    fn scene_model_keeps_grouped_unresolved_measurement_rows() {
        let molecule = MoleculeBuilder::new("source")
            .add_atom(poly_atom("A", "ALA", 1, "CA"), Vec3::new(0.0, 0.0, 0.0))
            .add_atom(poly_atom("A", "ALA", 2, "CA"), Vec3::new(2.0, 0.0, 0.0))
            .build();
        let first = AtomIndex(0);
        let second = AtomIndex(1);

        let mut registry = ObjectRegistry::default();
        registry.add(MoleculeObject::new(molecule));
        let mut measurement = MeasurementObject::new("d1", MeasurementKind::Distance);
        measurement
            .add_entry(MeasurementEntry::new(vec![
                MeasurementAnchor::new("source", first),
                MeasurementAnchor::new("source", second),
            ]))
            .expect("distance entry");
        registry.add(measurement);
        assert!(registry.add_to_group("measurements", "d1"));
        registry.remove("source");

        let mut model = SceneModel::new();
        let palette = ThemedPalette::dark();
        let ctx = SceneColorContext {
            named_palette: &NamedPalette::default(),
            palette: &palette,
            settings: &Settings::default(),
        };
        assert!(model.sync(&registry, &ctx));

        let group = model.get_group("measurements").expect("measurement group");
        assert_eq!(group.children.len(), 1);
        assert_eq!(group.children[0].name, "d1");
        assert!(group.children[0].has_unresolved_entities);
        assert!(!group.children[0].capabilities.focus);
        assert_eq!(
            group.children[0].focus_disabled_reason.as_deref(),
            Some("No resolvable anchors")
        );
    }

    #[test]
    fn scene_model_lists_top_level_and_grouped_labels_with_mixed_style_and_focus_state() {
        let molecule = MoleculeBuilder::new("source")
            .add_atom(poly_atom("A", "ALA", 1, "CA"), Vec3::new(0.0, 0.0, 0.0))
            .add_atom(poly_atom("A", "ALA", 2, "CA"), Vec3::new(2.0, 0.0, 0.0))
            .build();
        let first = AtomIndex(0);
        let second = AtomIndex(1);
        let mut first_style = LabelEntityPresentation::default();
        first_style
            .set_color(ColorIndex::Named(1))
            .expect("named label color");
        let mut second_style = LabelEntityPresentation::default();
        second_style
            .set_color(ColorIndex::Named(2))
            .expect("named label color");

        let mut registry = ObjectRegistry::default();
        registry.add(MoleculeObject::new(molecule));
        registry.add(LabelObject::with_entities(
            "top-labels",
            vec![
                LabelEntity::with_presentation(
                    AtomAnchor::new("source", first),
                    "first",
                    first_style,
                ),
                LabelEntity::with_presentation(
                    AtomAnchor::new("source", second),
                    "second",
                    second_style,
                ),
            ],
        ));
        registry.add(LabelObject::with_entities(
            "grouped-labels",
            vec![LabelEntity::new(
                AtomAnchor::new("source", first),
                "grouped",
            )],
        ));
        assert!(registry.add_to_group("annotations", "grouped-labels"));

        let mut model = SceneModel::new();
        let palette = ThemedPalette::dark();
        let settings = Settings::default();
        let named = NamedPalette::default();
        let ctx = SceneColorContext {
            named_palette: &named,
            palette: &palette,
            settings: &settings,
        };
        assert!(model.sync(&registry, &ctx));

        let top = model.get("top-labels").expect("top-level label row");
        assert_eq!(top.kind, SceneObjectKind::Label);
        assert_eq!(top.entity_count, 2);
        assert_eq!(top.color, SidebarColor::Multicolor);
        assert!(top.capabilities.focus);
        assert!(top.focus_disabled_reason.is_none());

        let group = model.get_group("annotations").expect("annotation group");
        assert_eq!(group.children[0].name, "grouped-labels");
        assert_eq!(group.children[0].kind, SceneObjectKind::Label);

        registry.remove("source");
        assert!(model.sync(&registry, &ctx));
        for name in ["top-labels", "grouped-labels"] {
            let row = model.get(name).expect("orphaned label remains listed");
            assert!(row.has_unresolved_entities);
            assert!(!row.capabilities.focus);
            assert_eq!(
                row.focus_disabled_reason.as_deref(),
                Some("No resolvable anchors")
            );
        }
    }

    // ── Selector-clause synthesis (build_selector_clause via real partitions) ──

    #[test]
    fn selector_clause_single_subchain_object_is_empty() {
        // Object holds exactly one subchain (single biopolymer chain).
        // The whole-object row covers everything → empty clause.
        let mut b = MoleculeBuilder::new("m");
        for resv in 1..=3 {
            b = b.add_atom(
                poly_atom("A", "ALA", resv, "CA"),
                Vec3::new(resv as f32, 0.0, 0.0),
            );
        }
        let mol = b.build();
        assert_eq!(clause_for(&mol, 0), "");
    }

    #[test]
    fn selector_clause_single_subchain_in_chain_drops_kind_qualifier() {
        // 1IGT-shaped: chain A polymer + chain E single glycan subchain.
        // Chain E entry should produce just `chain E` (no `and organic …`).
        let mut b = MoleculeBuilder::new("m");
        b = b.add_atom(poly_atom("A", "ALA", 1, "CA"), Vec3::new(0.0, 0.0, 0.0));
        b = b.add_atom(poly_atom("A", "ALA", 2, "CA"), Vec3::new(1.0, 0.0, 0.0));
        b = b.add_atom(
            organic_atom("E", "NAG", 1001, "C1"),
            Vec3::new(10.0, 0.0, 0.0),
        );
        let mol = b.build();
        // Entries are sorted (chain_id, min_atom_index): A first, E second.
        assert_eq!(clause_for(&mol, 0), "chain A");
        assert_eq!(clause_for(&mol, 1), "chain E");
    }

    #[test]
    fn selector_clause_blank_chain_is_quoted() {
        let mut b = MoleculeBuilder::new("m");
        b = b.add_atom(poly_atom("", "ALA", 1, "CA"), Vec3::new(0.0, 0.0, 0.0));
        b = b.add_atom(poly_atom("A", "ALA", 1, "CA"), Vec3::new(1.0, 0.0, 0.0));
        let mol = b.build();

        assert_eq!(clause_for(&mol, 0), "chain \"\"");
        assert_eq!(clause_for(&mol, 1), "chain A");
    }

    #[test]
    fn selector_clause_polymer_plus_organic_uses_resi_range() {
        // chain A: polymer (resv 1-2) + HEM ligand at resv 200.
        // Two subchains in chain A → polymer gets `polymer`, HEM gets
        // `organic and resi 200`.
        let mut b = MoleculeBuilder::new("m");
        b = b.add_atom(poly_atom("A", "ALA", 1, "CA"), Vec3::new(0.0, 0.0, 0.0));
        b = b.add_atom(poly_atom("A", "ALA", 2, "CA"), Vec3::new(1.0, 0.0, 0.0));
        b = b.add_atom(
            organic_atom("A", "HEM", 200, "FE"),
            Vec3::new(5.0, 0.0, 0.0),
        );
        let mol = b.build();
        assert_eq!(clause_for(&mol, 0), "chain A and polymer");
        assert_eq!(clause_for(&mol, 1), "chain A and organic and resi 200");
    }

    #[test]
    fn selector_clause_polymer_plus_ion_uses_resn() {
        // chain A: polymer + ZN ion → ion clause is `chain A and resn ZN`.
        let mut b = MoleculeBuilder::new("m");
        b = b.add_atom(poly_atom("A", "ALA", 1, "CA"), Vec3::new(0.0, 0.0, 0.0));
        b = b.add_atom(poly_atom("A", "ALA", 2, "CA"), Vec3::new(1.0, 0.0, 0.0));
        b = b.add_atom(inorganic_atom("A", "ZN", 300), Vec3::new(5.0, 0.0, 0.0));
        let mol = b.build();
        assert_eq!(clause_for(&mol, 0), "chain A and polymer");
        assert_eq!(clause_for(&mol, 1), "chain A and resn ZN");
    }

    #[test]
    fn selector_clause_solvent_uses_solvent_qualifier() {
        // Multi-chain: polymer chain A + waters in chain S.
        // Solvent in its own chain (single subchain there) → just `chain S`.
        let mut b = MoleculeBuilder::new("m");
        b = b.add_atom(poly_atom("A", "ALA", 1, "CA"), Vec3::new(0.0, 0.0, 0.0));
        b = b.add_atom(poly_atom("A", "ALA", 2, "CA"), Vec3::new(1.0, 0.0, 0.0));
        b = b.add_atom(solvent_atom("S", 1), Vec3::new(10.0, 0.0, 0.0));
        b = b.add_atom(solvent_atom("S", 2), Vec3::new(11.0, 0.0, 0.0));
        let mol = b.build();
        // Chain S has exactly one subchain → drops to `chain S`.
        assert_eq!(clause_for(&mol, 1), "chain S");
    }

    #[test]
    fn selector_clause_solvent_with_polymer_in_same_chain() {
        // Polymer + solvent both in chain A → solvent uses
        // `chain A and solvent`.
        let mut b = MoleculeBuilder::new("m");
        b = b.add_atom(poly_atom("A", "ALA", 1, "CA"), Vec3::new(0.0, 0.0, 0.0));
        b = b.add_atom(poly_atom("A", "ALA", 2, "CA"), Vec3::new(1.0, 0.0, 0.0));
        b = b.add_atom(solvent_atom("A", 100), Vec3::new(10.0, 0.0, 0.0));
        b = b.add_atom(solvent_atom("A", 101), Vec3::new(11.0, 0.0, 0.0));
        let mol = b.build();
        assert_eq!(clause_for(&mol, 0), "chain A and polymer");
        assert_eq!(clause_for(&mol, 1), "chain A and solvent");
    }

    #[test]
    fn selector_clause_multiple_ions_join_resns() {
        // Polymer + multiple ion species in same chain →
        // `chain A and resn ZN+CL` (or with whatever discovery order yields).
        let mut b = MoleculeBuilder::new("m");
        b = b.add_atom(poly_atom("A", "ALA", 1, "CA"), Vec3::new(0.0, 0.0, 0.0));
        b = b.add_atom(poly_atom("A", "ALA", 2, "CA"), Vec3::new(1.0, 0.0, 0.0));
        b = b.add_atom(inorganic_atom("A", "ZN", 300), Vec3::new(5.0, 0.0, 0.0));
        b = b.add_atom(inorganic_atom("A", "CL", 301), Vec3::new(6.0, 0.0, 0.0));
        let mol = b.build();
        // Polymer + (ZN, CL grouped together): ions of different resns can
        // either share an entry (Inorganic) or split. Verify the polymer
        // clause and that any ion entries reference inorganic resns.
        assert_eq!(clause_for(&mol, 0), "chain A and polymer");
        // Walk every non-polymer entry and ensure its clause is a valid
        // "chain A and resn …" form.
        for (idx, e) in mol
            .subchain_partition()
            .entries()
            .iter()
            .enumerate()
            .skip(1)
        {
            assert_eq!(e.chain_id.as_str(), "A");
            let clause = clause_for(&mol, idx as u32);
            assert!(clause.starts_with("chain A and resn "), "got: {}", clause);
        }
    }
}
