//! Selection Abstract Syntax Tree
//!
//! Defines the AST types for parsed selection expressions.

use crate::pattern::{IntSpec, Pattern, ResiSpec};

/// Comparison operators for numeric properties
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompareOp {
    /// Equal to
    Eq,
    /// Not equal to
    Ne,
    /// Less than
    Lt,
    /// Less than or equal to
    Le,
    /// Greater than
    Gt,
    /// Greater than or equal to
    Ge,
}

impl CompareOp {
    /// Apply the comparison to two f32 values
    #[inline]
    pub fn compare_f32(self, a: f32, b: f32) -> bool {
        match self {
            CompareOp::Eq => (a - b).abs() < f32::EPSILON,
            CompareOp::Ne => (a - b).abs() >= f32::EPSILON,
            CompareOp::Lt => a < b,
            CompareOp::Le => a <= b,
            CompareOp::Gt => a > b,
            CompareOp::Ge => a >= b,
        }
    }

    /// Apply the comparison to two i32 values
    #[inline]
    pub fn compare_i32(self, a: i32, b: i32) -> bool {
        match self {
            CompareOp::Eq => a == b,
            CompareOp::Ne => a != b,
            CompareOp::Lt => a < b,
            CompareOp::Le => a <= b,
            CompareOp::Gt => a > b,
            CompareOp::Ge => a >= b,
        }
    }
}

impl std::fmt::Display for CompareOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompareOp::Eq => write!(f, "="),
            CompareOp::Ne => write!(f, "!="),
            CompareOp::Lt => write!(f, "<"),
            CompareOp::Le => write!(f, "<="),
            CompareOp::Gt => write!(f, ">"),
            CompareOp::Ge => write!(f, ">="),
        }
    }
}

/// Specification for slash macro notation
///
/// Format: `/model/segi/chain/resn`resi/name`alt`
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MacroSpec {
    /// Model/object name (None = any)
    pub model: Option<Pattern>,
    /// Segment identifier (None = any)
    pub segi: Option<Pattern>,
    /// Chain identifier (None = any)
    pub chain: Option<Pattern>,
    /// Residue name (None = any)
    pub resn: Option<Pattern>,
    /// Residue identifier (None = any)
    pub resi: Option<ResiSpec>,
    /// Atom name (None = any)
    pub name: Option<Pattern>,
    /// Alternate location (None = any)
    pub alt: Option<Pattern>,
}

/// A selection expression AST node
///
/// This enum represents all possible selection expressions in the
/// Command selection algebra. The AST is built by the parser and evaluated by
/// the evaluator.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionExpr {
    // =========================================================================
    // Logical Operators
    // =========================================================================
    /// Logical AND of two selections
    And(Box<SelectionExpr>, Box<SelectionExpr>),

    /// Logical OR of two selections
    Or(Box<SelectionExpr>, Box<SelectionExpr>),

    /// Logical NOT (complement) of a selection
    Not(Box<SelectionExpr>),

    // =========================================================================
    // Property Selectors (1 argument - pattern)
    // =========================================================================
    /// Atom name (e.g., `name CA`, `name C*`)
    Name(Pattern),

    /// Residue name (e.g., `resn ALA+GLY`)
    Resn(Pattern),

    /// Residue identifier (e.g., `resi 100-200`)
    Resi(ResiSpec),

    /// Chain identifier (e.g., `chain A+B`)
    Chain(Pattern),

    /// Segment identifier (e.g., `segi PROA`)
    Segi(Pattern),

    /// Element symbol (e.g., `elem C+N+O`)
    Elem(Pattern),

    /// Alternate location (e.g., `alt A`)
    Alt(Pattern),

    /// Model/object name (e.g., `model protein`)
    Model(Pattern),

    /// Atom index (internal, 0-based)
    Index(IntSpec),

    /// PDB atom ID
    Id(IntSpec),

    /// Atom rank (order at load time)
    Rank(IntSpec),

    /// Secondary structure (e.g., `ss H+S`)
    SecondaryStructure(Pattern),

    /// State number
    State(IntSpec),

    /// Flag number
    Flag(IntSpec),

    /// Representation type (e.g., `rep cartoon`)
    Rep(Pattern),

    /// Color name or index
    Color(Pattern),

    /// Cartoon color
    CartoonColor(Pattern),

    /// Ribbon color
    RibbonColor(Pattern),

    /// Atom label
    Label(Pattern),

    /// Custom property
    Custom(Pattern),

    /// Text type
    TextType(Pattern),

    /// Numeric type
    NumericType(Pattern),

    /// Stereochemistry
    Stereo(Pattern),

    /// Peptide sequence (single-letter codes)
    PepSeq(Pattern),

    // =========================================================================
    // Numeric Comparisons (2 arguments - operator and value)
    // =========================================================================
    /// B-factor/temperature factor (e.g., `b > 50`)
    BFactor(CompareOp, f32),

    /// Occupancy (e.g., `q < 1`)
    Occupancy(CompareOp, f32),

    /// Partial charge (e.g., `pc. > 0`)
    PartialCharge(CompareOp, f32),

    /// Formal charge (e.g., `fc. = 0`)
    FormalCharge(CompareOp, i32),

    /// X coordinate (e.g., `x > 0`)
    X(CompareOp, f32),

    /// Y coordinate (e.g., `y < 10`)
    Y(CompareOp, f32),

    /// Z coordinate (e.g., `z >= 5`)
    Z(CompareOp, f32),

    // =========================================================================
    // Property access (p.<property>)
    // =========================================================================
    /// Generic property access with comparison
    Property(String, CompareOp, PropertyValue),

    // =========================================================================
    // Special Selections (0 arguments)
    // =========================================================================
    /// All atoms
    All,

    /// No atoms
    None,

    /// Visible atoms
    Visible,

    /// Enabled atoms
    Enabled,

    /// Hydrogen atoms
    Hydrogens,

    /// HETATM atoms
    Hetatm,

    /// Bonded atoms
    Bonded,

    /// Polymer atoms
    Polymer,

    /// Protein polymer atoms
    PolymerProtein,

    /// Nucleic acid polymer atoms
    PolymerNucleic,

    /// Organic molecules
    Organic,

    /// Inorganic molecules
    Inorganic,

    /// Solvent molecules
    Solvent,

    /// Metal atoms
    Metals,

    /// Backbone atoms
    Backbone,

    /// Sidechain atoms
    Sidechain,

    /// Hydrogen bond donors
    Donors,

    /// Hydrogen bond acceptors
    Acceptors,

    /// Delocalized atoms
    Delocalized,

    /// Fixed atoms
    Fixed,

    /// Restrained atoms
    Restrained,

    /// Masked atoms
    Masked,

    /// Protected atoms
    Protected,

    /// Atoms present in current state
    Present,

    /// Guide atoms (for cartoon)
    Guide,

    /// Origin point
    Origin,

    /// Center point
    Center,

    // =========================================================================
    // Distance-Based Operators (unary with distance parameter)
    // =========================================================================
    /// Select atoms within distance of selection (exclusive)
    /// `around 5 of selection` -> atoms within 5A of selection, not including selection
    Around(f32, Box<SelectionExpr>),

    /// Expand selection by distance (inclusive)
    /// `expand 5 by selection` -> selection + atoms within 5A
    Expand(f32, Box<SelectionExpr>),

    /// Extend selection by N bonds
    /// `extend 2 by selection` -> selection + atoms within 2 bonds
    Extend(u32, Box<SelectionExpr>),

    /// Gap distance (for surface calculations)
    Gap(f32, Box<SelectionExpr>),

    // =========================================================================
    // Distance Binary Operators (selection within/beyond X of selection)
    // =========================================================================
    /// `selection1 within 5 of selection2`
    Within(f32, Box<SelectionExpr>, Box<SelectionExpr>),

    /// `selection1 beyond 5 of selection2`
    Beyond(f32, Box<SelectionExpr>, Box<SelectionExpr>),

    /// `selection1 near_to 5 of selection2` (like within but different semantics)
    NearTo(f32, Box<SelectionExpr>, Box<SelectionExpr>),

    // =========================================================================
    // Grouping Operators
    // =========================================================================
    /// Select entire residues containing selection
    ByRes(Box<SelectionExpr>),

    /// Select entire chains containing selection
    ByChain(Box<SelectionExpr>),

    /// Select entire objects containing selection
    ByObject(Box<SelectionExpr>),

    /// Select entire molecules (connected components) containing selection
    ByMolecule(Box<SelectionExpr>),

    /// Select entire segments containing selection
    BySegment(Box<SelectionExpr>),

    /// Select entire fragments containing selection
    ByFragment(Box<SelectionExpr>),

    /// Select C-alpha atoms of residues containing selection
    ByCAlpha(Box<SelectionExpr>),

    /// Select rings containing selection
    ByRing(Box<SelectionExpr>),

    /// Select by unit cell
    ByCell(Box<SelectionExpr>),

    // =========================================================================
    // Neighbor Operators
    // =========================================================================
    /// Atoms bonded to selection (excludes original selection)
    Neighbor(Box<SelectionExpr>),

    /// Atoms bound to selection (includes original selection)
    BoundTo(Box<SelectionExpr>),

    // =========================================================================
    // Position Operators
    // =========================================================================
    /// First atom in selection
    First(Box<SelectionExpr>),

    /// Last atom in selection
    Last(Box<SelectionExpr>),

    // =========================================================================
    // Set Operators
    // =========================================================================
    /// Residue-level pattern matching
    /// `selection1 like selection2`
    Like(Box<SelectionExpr>, Box<SelectionExpr>),

    /// Membership test
    /// `selection1 in selection2`
    In(Box<SelectionExpr>, Box<SelectionExpr>),

    // =========================================================================
    // References
    // =========================================================================
    /// Named selection reference (e.g., `%sele1`)
    Selection(String),

    /// Slash macro notation (e.g., `/protein//A/ALA/CA`)
    Macro(MacroSpec),
}

/// Property value for generic property access
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// String value
    String(String),
    /// Integer value
    Int(i32),
    /// Float value
    Float(f32),
}

impl SelectionExpr {
    /// Create an AND expression
    pub fn and(self, other: SelectionExpr) -> SelectionExpr {
        SelectionExpr::And(Box::new(self), Box::new(other))
    }

    /// Create an OR expression
    pub fn or(self, other: SelectionExpr) -> SelectionExpr {
        SelectionExpr::Or(Box::new(self), Box::new(other))
    }

    /// Create a NOT expression
    pub fn negate(self) -> SelectionExpr {
        SelectionExpr::Not(Box::new(self))
    }

    /// Collect all `Selection(name)` references in this expression tree.
    pub fn selection_references(&self) -> Vec<&str> {
        let mut refs = Vec::new();
        self.collect_selection_refs(&mut refs);
        refs
    }

    /// Returns whether evaluation needs semantic atom-label text.
    pub fn uses_atom_labels(&self) -> bool {
        match self {
            SelectionExpr::Label(_) => true,
            SelectionExpr::And(left, right)
            | SelectionExpr::Or(left, right)
            | SelectionExpr::Like(left, right)
            | SelectionExpr::In(left, right)
            | SelectionExpr::Within(_, left, right)
            | SelectionExpr::Beyond(_, left, right)
            | SelectionExpr::NearTo(_, left, right) => {
                left.uses_atom_labels() || right.uses_atom_labels()
            }
            SelectionExpr::Not(inner)
            | SelectionExpr::ByRes(inner)
            | SelectionExpr::ByChain(inner)
            | SelectionExpr::ByObject(inner)
            | SelectionExpr::ByMolecule(inner)
            | SelectionExpr::BySegment(inner)
            | SelectionExpr::ByFragment(inner)
            | SelectionExpr::ByCAlpha(inner)
            | SelectionExpr::ByRing(inner)
            | SelectionExpr::ByCell(inner)
            | SelectionExpr::Neighbor(inner)
            | SelectionExpr::BoundTo(inner)
            | SelectionExpr::First(inner)
            | SelectionExpr::Last(inner)
            | SelectionExpr::Around(_, inner)
            | SelectionExpr::Expand(_, inner)
            | SelectionExpr::Extend(_, inner)
            | SelectionExpr::Gap(_, inner) => inner.uses_atom_labels(),
            _ => false,
        }
    }

    fn collect_selection_refs<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            SelectionExpr::Selection(name) => out.push(name),
            SelectionExpr::And(l, r)
            | SelectionExpr::Or(l, r)
            | SelectionExpr::Like(l, r)
            | SelectionExpr::In(l, r)
            | SelectionExpr::Within(_, l, r)
            | SelectionExpr::Beyond(_, l, r)
            | SelectionExpr::NearTo(_, l, r) => {
                l.collect_selection_refs(out);
                r.collect_selection_refs(out);
            }
            SelectionExpr::Not(inner)
            | SelectionExpr::ByRes(inner)
            | SelectionExpr::ByChain(inner)
            | SelectionExpr::ByObject(inner)
            | SelectionExpr::ByMolecule(inner)
            | SelectionExpr::BySegment(inner)
            | SelectionExpr::ByFragment(inner)
            | SelectionExpr::ByCAlpha(inner)
            | SelectionExpr::ByRing(inner)
            | SelectionExpr::ByCell(inner)
            | SelectionExpr::Neighbor(inner)
            | SelectionExpr::BoundTo(inner)
            | SelectionExpr::First(inner)
            | SelectionExpr::Last(inner)
            | SelectionExpr::Around(_, inner)
            | SelectionExpr::Expand(_, inner)
            | SelectionExpr::Extend(_, inner)
            | SelectionExpr::Gap(_, inner) => {
                inner.collect_selection_refs(out);
            }
            // All other variants (keywords, properties, etc.) have no sub-expressions
            _ => {}
        }
    }

    /// Check if this is a simple expression (no children)
    pub fn is_simple(&self) -> bool {
        matches!(
            self,
            SelectionExpr::All
                | SelectionExpr::None
                | SelectionExpr::Visible
                | SelectionExpr::Enabled
                | SelectionExpr::Hydrogens
                | SelectionExpr::Hetatm
                | SelectionExpr::Bonded
                | SelectionExpr::Polymer
                | SelectionExpr::PolymerProtein
                | SelectionExpr::PolymerNucleic
                | SelectionExpr::Organic
                | SelectionExpr::Inorganic
                | SelectionExpr::Solvent
                | SelectionExpr::Metals
                | SelectionExpr::Backbone
                | SelectionExpr::Sidechain
                | SelectionExpr::Donors
                | SelectionExpr::Acceptors
                | SelectionExpr::Delocalized
                | SelectionExpr::Fixed
                | SelectionExpr::Restrained
                | SelectionExpr::Masked
                | SelectionExpr::Protected
                | SelectionExpr::Present
                | SelectionExpr::Guide
                | SelectionExpr::Origin
                | SelectionExpr::Center
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_op_f32() {
        assert!(CompareOp::Gt.compare_f32(5.0, 3.0));
        assert!(!CompareOp::Gt.compare_f32(3.0, 5.0));
        assert!(CompareOp::Eq.compare_f32(3.0, 3.0));
        assert!(CompareOp::Le.compare_f32(3.0, 5.0));
    }

    #[test]
    fn test_compare_op_i32() {
        assert!(CompareOp::Eq.compare_i32(5, 5));
        assert!(CompareOp::Ne.compare_i32(5, 3));
        assert!(CompareOp::Lt.compare_i32(3, 5));
    }

    #[test]
    fn detects_nested_atom_label_dependencies() {
        let label = SelectionExpr::Label(Pattern::Exact("active".to_string()));
        assert!(label.clone().negate().uses_atom_labels());
        assert!(SelectionExpr::All.and(label).uses_atom_labels());
        assert!(!SelectionExpr::All
            .and(SelectionExpr::Name(Pattern::Exact("CA".to_string())))
            .uses_atom_labels());
    }

    #[test]
    fn test_selection_expr_combinators() {
        let expr1 = SelectionExpr::All;
        let expr2 = SelectionExpr::Hydrogens;

        let combined = expr1.clone().and(expr2.clone());
        assert!(matches!(combined, SelectionExpr::And(_, _)));

        let combined = expr1.or(expr2);
        assert!(matches!(combined, SelectionExpr::Or(_, _)));

        let negated = SelectionExpr::All.negate();
        assert!(matches!(negated, SelectionExpr::Not(_)));
    }

    #[test]
    fn test_is_simple() {
        assert!(SelectionExpr::All.is_simple());
        assert!(SelectionExpr::Hydrogens.is_simple());
        assert!(!SelectionExpr::Not(Box::new(SelectionExpr::All)).is_simple());
    }
}
