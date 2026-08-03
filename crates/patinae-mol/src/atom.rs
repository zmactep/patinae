//! Atom data structure
//!
//! Provides the `Atom` struct containing all atom properties (PDB/mmCIF specification).
//!
//! The atom data is organized into logical components:
//! - [`AtomResidue`] - Shared residue information (chain, resn, resv, etc.)
//! - [`AtomState`] - Per-atom chemical/bonding state flags
//! - [`AtomRepresentation`] - Display/visualization settings
//! - [`Atom`] - Core identity and physical/chemical properties

use serde::{Deserialize, Serialize};

use crate::element::Element;
use crate::flags::{AtomFlags, AtomGeometry, Chirality, Stereo};
use crate::residue::ResidueKey;
use crate::secondary::SecondaryStructure;
use std::ops::Deref;
use std::sync::Arc;

/// Representation visibility flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct RepMask(pub u32);

impl RepMask {
    /// No representations visible
    pub const NONE: RepMask = RepMask(0);
    /// All representations visible
    pub const ALL: RepMask = RepMask(u32::MAX);

    // Representation bit positions (stable for PSE session format compatibility)
    /// Sticks (cylinders) representation
    pub const STICKS: RepMask = RepMask(1 << 0);
    /// Spheres representation
    pub const SPHERES: RepMask = RepMask(1 << 1);
    /// Surface representation
    pub const SURFACE: RepMask = RepMask(1 << 2);
    /// Labels representation
    pub const LABELS: RepMask = RepMask(1 << 3);
    /// Non-bonded spheres representation
    pub const NONBONDED: RepMask = RepMask(1 << 4);
    /// Cartoon representation
    pub const CARTOON: RepMask = RepMask(1 << 5);
    /// Ribbon representation
    pub const RIBBON: RepMask = RepMask(1 << 6);
    /// Lines representation
    pub const LINES: RepMask = RepMask(1 << 7);
    /// Mesh representation
    pub const MESH: RepMask = RepMask(1 << 8);
    /// Dots representation
    pub const DOTS: RepMask = RepMask(1 << 9);
    /// Dashes representation
    pub const DASHES: RepMask = RepMask(1 << 10);
    /// Cell representation
    pub const CELL: RepMask = RepMask(1 << 11);
    /// CGO representation
    pub const CGO: RepMask = RepMask(1 << 12);
    /// Callback representation
    pub const CALLBACK: RepMask = RepMask(1 << 13);
    /// Extent representation
    pub const EXTENT: RepMask = RepMask(1 << 14);
    /// Slice representation
    pub const SLICE: RepMask = RepMask(1 << 15);
    /// Anisotropic-thermal-displacement (ADP) ellipsoid representation
    pub const ELLIPSOIDS: RepMask = RepMask(1 << 16);
    /// Reps whose whole-object show/hide can use an object draw mask.
    pub const DRAW_MASK_REPS: RepMask =
        RepMask(Self::SURFACE.0 | Self::MESH.0 | Self::CARTOON.0 | Self::RIBBON.0);

    /// Check if a representation is visible
    #[inline]
    pub fn is_visible(&self, rep: RepMask) -> bool {
        (self.0 & rep.0) != 0
    }

    /// Set a representation visible
    #[inline]
    pub fn set_visible(&mut self, rep: RepMask) {
        self.0 |= rep.0;
    }

    /// Set a representation hidden
    #[inline]
    pub fn set_hidden(&mut self, rep: RepMask) {
        self.0 &= !rep.0;
    }

    /// Toggle a representation
    #[inline]
    pub fn toggle(&mut self, rep: RepMask) {
        self.0 ^= rep.0;
    }

    /// Combine two masks (union)
    #[inline]
    pub const fn union(self, other: RepMask) -> RepMask {
        RepMask(self.0 | other.0)
    }

    /// Intersect two masks
    #[inline]
    pub const fn intersection(self, other: RepMask) -> RepMask {
        RepMask(self.0 & other.0)
    }

    /// Remove every bit present in `other`.
    #[inline]
    pub const fn difference(self, other: RepMask) -> RepMask {
        RepMask(self.0 & !other.0)
    }

    /// True if no representation bits are set.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True if every set bit in this mask is also set in `other`.
    #[inline]
    pub const fn is_subset_of(self, other: RepMask) -> bool {
        (self.0 & !other.0) == 0
    }

    /// True if a whole-object toggle can skip per-atom rep mutation.
    #[inline]
    pub const fn can_toggle_with_draw_mask(self) -> bool {
        !self.is_empty() && self.is_subset_of(Self::DRAW_MASK_REPS)
    }
}

/// Sentinel value indicating "no color override" (use base color)
pub const COLOR_UNSET: i32 = i32::MIN;

/// Color by element (CPK coloring)
pub const COLOR_BY_ELEMENT: i32 = -1;

/// Color by chain
pub const COLOR_BY_CHAIN: i32 = -2;

/// Per-atom color settings for different representations
///
/// Contains the base color index and optional per-representation color overrides.
/// When a representation-specific color is `COLOR_UNSET`, the base color is used instead.
/// Uses sentinel values instead of `Option<i32>` for memory efficiency (4 bytes vs 8 bytes per field).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AtomColors {
    /// Base color index (resolved via patinae-color crate)
    /// -1 = by element, -2 = by chain, -3 = by ss, -4 = by b-factor
    pub base: i32,

    /// Cartoon-specific color (COLOR_UNSET = use base)
    pub cartoon: i32,

    /// Ribbon-specific color (COLOR_UNSET = use base)
    pub ribbon: i32,

    /// Stick-specific color (COLOR_UNSET = use base)
    pub stick: i32,

    /// Line-specific color (COLOR_UNSET = use base)
    pub line: i32,

    /// Surface-specific color (COLOR_UNSET = use base)
    pub surface: i32,

    /// Mesh-specific color (COLOR_UNSET = use base)
    pub mesh: i32,

    /// Sphere-specific color (COLOR_UNSET = use base)
    pub sphere: i32,

    /// Dot-specific color (COLOR_UNSET = use base)
    pub dot: i32,

    /// Ellipsoid-specific color (COLOR_UNSET = use base)
    pub ellipsoid: i32,
}

impl Default for AtomColors {
    fn default() -> Self {
        AtomColors {
            base: COLOR_BY_ELEMENT,
            cartoon: COLOR_BY_CHAIN,
            ribbon: COLOR_UNSET,
            stick: COLOR_UNSET,
            line: COLOR_UNSET,
            surface: COLOR_UNSET,
            mesh: COLOR_UNSET,
            sphere: COLOR_UNSET,
            dot: COLOR_UNSET,
            ellipsoid: COLOR_UNSET,
        }
    }
}

impl AtomColors {
    /// Create a new AtomColors with default values (CPK base, cartoon by chain)
    pub fn new() -> Self {
        Self::default()
    }

    /// Create AtomColors with a specific base color index
    pub fn with_base(base: i32) -> Self {
        AtomColors {
            base,
            ..Default::default()
        }
    }

    /// Get the effective color for cartoon representation
    #[inline]
    pub fn cartoon_or_base(&self) -> i32 {
        if self.cartoon != COLOR_UNSET {
            self.cartoon
        } else {
            self.base
        }
    }

    /// Get the effective color for ribbon representation
    #[inline]
    pub fn ribbon_or_base(&self) -> i32 {
        if self.ribbon != COLOR_UNSET {
            self.ribbon
        } else {
            self.base
        }
    }

    /// Get the effective color for stick representation
    #[inline]
    pub fn stick_or_base(&self) -> i32 {
        if self.stick != COLOR_UNSET {
            self.stick
        } else {
            self.base
        }
    }

    /// Get the effective color for line representation
    #[inline]
    pub fn line_or_base(&self) -> i32 {
        if self.line != COLOR_UNSET {
            self.line
        } else {
            self.base
        }
    }

    /// Get the effective color for surface representation
    #[inline]
    pub fn surface_or_base(&self) -> i32 {
        if self.surface != COLOR_UNSET {
            self.surface
        } else {
            self.base
        }
    }

    /// Get the effective color for mesh representation
    #[inline]
    pub fn mesh_or_base(&self) -> i32 {
        if self.mesh != COLOR_UNSET {
            self.mesh
        } else {
            self.base
        }
    }

    /// Get the effective color for sphere representation
    #[inline]
    pub fn sphere_or_base(&self) -> i32 {
        if self.sphere != COLOR_UNSET {
            self.sphere
        } else {
            self.base
        }
    }

    /// Get the effective color for dot representation
    #[inline]
    pub fn dot_or_base(&self) -> i32 {
        if self.dot != COLOR_UNSET {
            self.dot
        } else {
            self.base
        }
    }

    /// Get the effective color for ellipsoid representation
    #[inline]
    pub fn ellipsoid_or_base(&self) -> i32 {
        if self.ellipsoid != COLOR_UNSET {
            self.ellipsoid
        } else {
            self.base
        }
    }
}

// =============================================================================
// AtomResidue - Shared residue information
// =============================================================================

/// Residue information shared by atoms in the same residue
///
/// This struct composes [`ResidueKey`] (identity fields) with the segment identifier.
/// It implements [`Deref`] to [`ResidueKey`] for transparent field access.
///
/// Note: Secondary structure type (`ss_type`) is stored on the [`Atom`] directly,
/// not in `AtomResidue`, because it can be assigned/modified per-atom by the DSSP algorithm.
///
/// # Example
/// ```
/// use patinae_mol::{AtomResidue, ResidueKey};
///
/// let residue = AtomResidue::new(
///     ResidueKey::new("A", "ALA", 1, ' '),
///     String::new(),
/// );
///
/// // Access via Deref
/// assert_eq!(residue.chain, "A");
/// assert_eq!(residue.resn, "ALA");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AtomResidue {
    /// Identity key (chain, resn, resv, inscode) - used for lookups
    pub key: ResidueKey,
    /// Segment identifier
    pub segi: String,
}

impl Deref for AtomResidue {
    type Target = ResidueKey;

    fn deref(&self) -> &Self::Target {
        &self.key
    }
}

impl Default for AtomResidue {
    fn default() -> Self {
        AtomResidue {
            key: ResidueKey::new("", "", 0, ' '),
            segi: String::new(),
        }
    }
}

impl AtomResidue {
    /// Create a new AtomResidue
    pub fn new(key: ResidueKey, segi: String) -> Self {
        AtomResidue { key, segi }
    }

    /// Create a new AtomResidue from individual fields
    pub fn from_parts(
        chain: impl Into<String>,
        resn: impl Into<String>,
        resv: i32,
        inscode: char,
        segi: impl Into<String>,
    ) -> Self {
        AtomResidue {
            key: ResidueKey::new(chain, resn, resv, inscode),
            segi: segi.into(),
        }
    }
}

// =============================================================================
// AtomState - Per-atom chemical/bonding state flags
// =============================================================================

/// Per-atom state flags for chemical properties and bonding
///
/// Contains flags that describe the atom's chemical state and classification.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomState {
    /// Atom flags (protein, solvent, organic, etc.)
    pub flags: AtomFlags,
    /// Whether this is a HETATM (heteroatom)
    pub hetatm: bool,
    /// Whether this atom is bonded to any other atom
    pub bonded: bool,
    /// Hydrogen bond donor
    pub hb_donor: bool,
    /// Hydrogen bond acceptor
    pub hb_acceptor: bool,
}

// =============================================================================
// AtomRepresentation - Display/visualization state
// =============================================================================

/// Display/visualization state for an atom
///
/// Contains all settings related to how the atom is displayed,
/// including colors, visibility, labels, and per-atom settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomRepresentation {
    /// Color settings for different representations
    pub colors: AtomColors,
    /// Sphere-specific scale factor (None = use global setting)
    pub sphere_scale: Option<f32>,
    /// Per-atom sphere transparency override (None = use object/global setting)
    pub sphere_transparency: Option<f32>,
    /// Per-atom stick transparency override (None = use object/global setting)
    pub stick_transparency: Option<f32>,
    /// Per-atom cartoon transparency override (None = use object/global setting)
    pub cartoon_transparency: Option<f32>,
    /// Per-atom surface transparency override (None = use object/global setting)
    pub surface_transparency: Option<f32>,
    /// Per-atom ellipsoid transparency override (None = use object/global setting)
    pub ellipsoid_transparency: Option<f32>,
    /// Visible representations bitmask
    pub visible_reps: RepMask,
    /// Cartoon override (0 = auto/default based on ss_type)
    pub cartoon: i8,
    /// Text type label
    pub text_type: String,
    /// Custom label
    pub label: String,
    /// Whether this atom is masked (hidden from selection)
    pub masked: bool,
    /// Unique ID for per-atom settings (None if no custom settings)
    pub unique_id: Option<i32>,
    /// Whether this atom has per-atom settings
    pub has_setting: bool,
}

impl Default for AtomRepresentation {
    fn default() -> Self {
        AtomRepresentation {
            colors: AtomColors::default(),
            sphere_scale: None,
            sphere_transparency: None,
            stick_transparency: None,
            cartoon_transparency: None,
            surface_transparency: None,
            ellipsoid_transparency: None,
            visible_reps: RepMask::LINES,
            cartoon: 0,
            text_type: String::new(),
            label: String::new(),
            masked: false,
            unique_id: None,
            has_setting: false,
        }
    }
}

// =============================================================================
// Atom - Main atom data structure
// =============================================================================

/// Atom data structure
///
/// Contains all properties of an atom including identity, residue information,
/// physical and chemical properties, and display state.
///
/// # Structure
///
/// The atom data is organized into logical components:
/// - Core identity fields (name, element) are stored directly
/// - Residue information is shared via [`Arc<AtomResidue>`] for memory efficiency
/// - Chemical state flags are grouped in [`AtomState`]
/// - Display settings are grouped in [`AtomRepresentation`]
///
/// # Example
/// ```
/// use patinae_mol::{Atom, Element};
///
/// let atom = Atom::new("CA", Element::Carbon);
/// assert_eq!(&*atom.name, "CA");
/// assert_eq!(atom.element, Element::Carbon);
///
/// // Access residue info via Deref
/// assert_eq!(atom.residue.resn, "");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Atom {
    // =========================================================================
    // Identity
    // =========================================================================
    /// Atom name (e.g., "CA", "N", "O")
    /// Uses `Arc<str>` for memory efficiency - atom names repeat frequently
    pub name: Arc<str>,

    /// Chemical element
    pub element: Element,

    // =========================================================================
    // Per-atom residue field (varies within residue)
    // =========================================================================
    /// Alternate location indicator
    pub alt: char,

    // =========================================================================
    // Physical Properties
    // =========================================================================
    /// B-factor (temperature factor / atomic displacement parameter)
    pub b_factor: f32,

    /// Occupancy (0.0 to 1.0)
    pub occupancy: f32,

    /// Van der Waals radius in Angstroms
    pub vdw: f32,

    /// Partial charge
    pub partial_charge: f32,

    /// Formal charge (typically -2 to +2)
    pub formal_charge: i8,

    /// Electrostatic radius (for Poisson-Boltzmann calculations)
    pub elec_radius: f32,

    /// Anisotropic temperature factors (U11, U22, U33, U12, U13, U23)
    /// Only allocated when present in the structure
    pub anisou: Option<[f32; 6]>,

    // =========================================================================
    // Chemical Properties
    // =========================================================================
    /// Valence (total degree = number of bonds including implicit hydrogens)
    pub valence: u8,

    /// Geometry (tetrahedral, planar, linear, etc.)
    pub geom: AtomGeometry,

    /// SDF stereochemistry (parity)
    pub stereo: Stereo,

    /// R/S chirality
    pub chirality: Chirality,

    // =========================================================================
    // Secondary Structure
    // =========================================================================
    /// Secondary structure type (assigned by DSSP or from file)
    /// Note: Stored per-atom because DSSP algorithm can assign it independently
    pub ss_type: SecondaryStructure,

    // =========================================================================
    // Internal/Identity
    // =========================================================================
    /// PDB atom serial number
    pub id: i32,

    /// Rank for ordering
    pub rank: i32,

    /// Discrete state index (for discrete objects where each state has independent atoms)
    /// Value of 0 means not discrete, >0 means state index + 1
    pub discrete_state: i32,

    // =========================================================================
    // Composed Structs
    // =========================================================================
    /// Residue information (shared via Rc for memory efficiency)
    pub residue: Arc<AtomResidue>,

    /// Chemical/bonding state flags
    pub state: AtomState,

    /// Display/visualization settings
    pub repr: AtomRepresentation,
}

impl Default for Atom {
    fn default() -> Self {
        Atom {
            name: Arc::from(""),
            element: Element::Unknown,
            alt: ' ',
            b_factor: 0.0,
            occupancy: 1.0,
            vdw: 0.0, // Will be set from element if 0
            partial_charge: 0.0,
            formal_charge: 0,
            elec_radius: 0.0,
            anisou: None,
            valence: 0,
            geom: AtomGeometry::None,
            stereo: Stereo::None,
            chirality: Chirality::None,
            ss_type: SecondaryStructure::Loop,
            id: 0,
            rank: 0,
            discrete_state: 0,
            residue: Arc::new(AtomResidue::default()),
            state: AtomState::default(),
            repr: AtomRepresentation::default(),
        }
    }
}

impl Atom {
    /// Create a new atom with the given name and element
    pub fn new(name: impl Into<String>, element: Element) -> Self {
        Atom {
            name: Arc::from(name.into()),
            element,
            vdw: element.vdw_radius(),
            ..Default::default()
        }
    }

    /// Create a new atom from element symbol
    pub fn from_symbol(name: impl Into<String>, symbol: &str) -> Self {
        let element = Element::from_symbol(symbol).unwrap_or(Element::Unknown);
        Self::new(name, element)
    }

    /// Get the effective VdW radius (from atom or element default)
    #[inline]
    pub fn effective_vdw(&self) -> f32 {
        if self.vdw > 0.0 {
            self.vdw
        } else {
            self.element.vdw_radius()
        }
    }

    /// Get the covalent radius in Angstroms (from element default)
    #[inline]
    pub fn cov_radius(&self) -> f32 {
        self.element.cov_radius()
    }

    /// Get the atomic number
    #[inline]
    pub fn atomic_number(&self) -> u8 {
        self.element.atomic_number()
    }

    /// Check if this atom is hydrogen
    #[inline]
    pub fn is_hydrogen(&self) -> bool {
        self.element.is_hydrogen()
    }

    /// Check if this atom is a heavy atom (not hydrogen)
    #[inline]
    pub fn is_heavy(&self) -> bool {
        !self.is_hydrogen() && !self.element.is_unknown()
    }

    /// Check if this atom is carbon
    #[inline]
    pub fn is_carbon(&self) -> bool {
        self.element.is_carbon()
    }

    /// Check if this is a C-alpha (backbone) atom
    #[inline]
    pub fn is_ca(&self) -> bool {
        &*self.name == "CA" && self.element.is_carbon()
    }

    /// Check if this is a backbone atom (N, CA, C, O)
    pub fn is_backbone(&self) -> bool {
        matches!(&*self.name, "N" | "CA" | "C" | "O")
            && self.state.flags.contains(AtomFlags::PROTEIN)
    }

    /// Check if this is a side chain atom
    pub fn is_sidechain(&self) -> bool {
        self.state.flags.contains(AtomFlags::PROTEIN) && !self.is_backbone()
    }

    /// Set the residue information (creates a new AtomResidue)
    pub fn set_residue(&mut self, resn: impl Into<String>, resv: i32, chain: impl Into<String>) {
        self.residue = Arc::new(AtomResidue::from_parts(chain, resn, resv, ' ', ""));
    }
}

impl std::fmt::Display for Atom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Atom({} {} {} {}{})",
            self.name,
            self.element.symbol(),
            self.residue.chain,
            self.residue.resn,
            self.residue.resv
        )
    }
}

/// Builder for creating atoms with a fluent interface
#[derive(Debug, Default)]
pub struct AtomBuilder {
    atom: Atom,
    // Temporary residue fields for building
    chain: String,
    resn: String,
    resv: i32,
    inscode: char,
    segi: String,
}

impl AtomBuilder {
    /// Create a new atom builder
    pub fn new() -> Self {
        AtomBuilder {
            atom: Atom::default(),
            chain: String::new(),
            resn: String::new(),
            resv: 0,
            inscode: ' ',
            segi: String::new(),
        }
    }

    /// Set the atom name
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.atom.name = Arc::from(name.into());
        self
    }

    /// Set the element
    pub fn element(mut self, element: Element) -> Self {
        self.atom.element = element;
        if self.atom.vdw == 0.0 {
            self.atom.vdw = element.vdw_radius();
        }
        self
    }

    /// Set the element from symbol
    pub fn element_symbol(mut self, symbol: &str) -> Self {
        if let Some(element) = Element::from_symbol(symbol) {
            self.atom.element = element;
            if self.atom.vdw == 0.0 {
                self.atom.vdw = element.vdw_radius();
            }
        }
        self
    }

    /// Set residue name
    pub fn resn(mut self, resn: impl Into<String>) -> Self {
        self.resn = resn.into();
        self
    }

    /// Set residue number
    pub fn resv(mut self, resv: i32) -> Self {
        self.resv = resv;
        self
    }

    /// Set chain
    pub fn chain(mut self, chain: impl Into<String>) -> Self {
        self.chain = chain.into();
        self
    }

    /// Set insertion code
    pub fn inscode(mut self, inscode: char) -> Self {
        self.inscode = inscode;
        self
    }

    /// Set segment identifier
    pub fn segi(mut self, segi: impl Into<String>) -> Self {
        self.segi = segi.into();
        self
    }

    /// Set secondary structure type
    pub fn ss_type(mut self, ss_type: SecondaryStructure) -> Self {
        self.atom.ss_type = ss_type;
        self
    }

    /// Set B-factor
    pub fn b_factor(mut self, b: f32) -> Self {
        self.atom.b_factor = b;
        self
    }

    /// Set occupancy
    pub fn occupancy(mut self, q: f32) -> Self {
        self.atom.occupancy = q;
        self
    }

    /// Set formal charge
    pub fn formal_charge(mut self, charge: i8) -> Self {
        self.atom.formal_charge = charge;
        self
    }

    /// Set as HETATM
    pub fn hetatm(mut self, hetatm: bool) -> Self {
        self.atom.state.hetatm = hetatm;
        self
    }

    /// Set PDB serial number
    pub fn id(mut self, id: i32) -> Self {
        self.atom.id = id;
        self
    }

    /// Set atom flags
    pub fn flags(mut self, flags: AtomFlags) -> Self {
        self.atom.state.flags = flags;
        self
    }

    /// Set a shared residue (for memory efficiency)
    pub fn residue(mut self, residue: Arc<AtomResidue>) -> Self {
        self.atom.residue = residue;
        self
    }

    /// Build the atom
    pub fn build(mut self) -> Atom {
        // Only create a new residue if one wasn't explicitly set
        // and if any residue fields were modified
        if Arc::strong_count(&self.atom.residue) == 1 {
            // Default residue, check if we need to update it
            if !self.chain.is_empty()
                || !self.resn.is_empty()
                || self.resv != 0
                || self.inscode != ' '
                || !self.segi.is_empty()
            {
                self.atom.residue = Arc::new(AtomResidue::from_parts(
                    self.chain,
                    self.resn,
                    self.resv,
                    self.inscode,
                    self.segi,
                ));
            }
        }
        self.atom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atom_new() {
        let atom = Atom::new("CA", Element::Carbon);
        assert_eq!(&*atom.name, "CA");
        assert_eq!(atom.element, Element::Carbon);
        assert!((atom.vdw - 1.70).abs() < 0.01);
    }

    #[test]
    fn test_atom_from_symbol() {
        let atom = Atom::from_symbol("N", "N");
        assert_eq!(atom.element, Element::Nitrogen);
    }

    #[test]
    fn test_atom_builder() {
        let atom = AtomBuilder::new()
            .name("CA")
            .element(Element::Carbon)
            .resn("ALA")
            .resv(1)
            .chain("A")
            .b_factor(20.0)
            .build();

        assert_eq!(&*atom.name, "CA");
        assert_eq!(atom.residue.resn, "ALA");
        assert_eq!(atom.residue.resv, 1);
        assert_eq!(atom.residue.chain, "A");
        assert_eq!(atom.b_factor, 20.0);
    }

    #[test]
    fn test_atom_classification() {
        let mut atom = Atom::new("CA", Element::Carbon);
        atom.state.flags = AtomFlags::PROTEIN;
        assert!(atom.is_backbone());

        atom.name = Arc::from("CB");
        assert!(atom.is_sidechain());
    }

    #[test]
    fn test_atom_display() {
        let mut atom = Atom::new("CA", Element::Carbon);
        atom.residue = Arc::new(AtomResidue::from_parts("A", "ALA", 1, ' ', ""));
        assert_eq!(format!("{}", atom), "Atom(CA C A ALA1)");
    }

    #[test]
    fn test_rep_mask() {
        let mut mask = RepMask::NONE;
        assert!(!mask.is_visible(RepMask::CARTOON));

        mask.set_visible(RepMask::CARTOON);
        assert!(mask.is_visible(RepMask::CARTOON));

        mask.set_hidden(RepMask::CARTOON);
        assert!(!mask.is_visible(RepMask::CARTOON));
    }

    #[test]
    fn test_atom_residue_deref() {
        let residue = AtomResidue::from_parts("A", "ALA", 1, ' ', "SEG1");
        // Test Deref to ResidueKey fields
        assert_eq!(residue.chain, "A");
        assert_eq!(residue.resn, "ALA");
        assert_eq!(residue.resv, 1);
        assert_eq!(residue.inscode, ' ');
        // Test direct fields
        assert_eq!(residue.segi, "SEG1");
    }

    #[test]
    fn test_atom_state_default() {
        let state = AtomState::default();
        assert_eq!(state.flags, AtomFlags::empty());
        assert!(!state.hetatm);
        assert!(!state.bonded);
        assert!(!state.hb_donor);
        assert!(!state.hb_acceptor);
    }

    #[test]
    fn test_atom_representation_default() {
        let repr = AtomRepresentation::default();
        assert_eq!(repr.colors.base, COLOR_BY_ELEMENT);
        assert!(repr.visible_reps.is_visible(RepMask::LINES));
        assert!(!repr.masked);
    }

    #[test]
    fn test_shared_residue() {
        let residue = Arc::new(AtomResidue::from_parts("A", "ALA", 1, ' ', ""));

        let mut atom1 = Atom::new("N", Element::Nitrogen);
        atom1.residue = residue.clone();
        atom1.ss_type = SecondaryStructure::Helix;

        let mut atom2 = Atom::new("CA", Element::Carbon);
        atom2.residue = residue.clone();
        atom2.ss_type = SecondaryStructure::Helix;

        // Both atoms share the same residue
        assert!(Arc::ptr_eq(&atom1.residue, &atom2.residue));
        assert_eq!(Arc::strong_count(&residue), 3);
        // But ss_type is per-atom
        assert_eq!(atom1.ss_type, SecondaryStructure::Helix);
    }
}
