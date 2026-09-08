//! Keyword definitions for the selection language
//!
//! Defines all keywords recognized by the selection language,
//! including their aliases and operator types.

use phf::phf_map;

/// Keyword type indicating the operator category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeywordType {
    /// Zero-argument selection (e.g., `all`, `none`, `hydrogens`)
    Sel0,
    /// One-argument property selector (e.g., `name`, `resn`, `chain`)
    Sel1,
    /// Two-argument numeric comparison (e.g., `b`, `q`, `x`)
    Sel2,
    /// Three-argument property access (e.g., `p.`)
    Sel3,
    /// Unary prefix operator (e.g., `not`, `byres`)
    Opr1,
    /// Binary operator (e.g., `and`, `or`)
    Opr2,
    /// Distance operator with "of" (e.g., `within`, `beyond`)
    Op22,
    /// One-argument distance operator (e.g., `around`, `expand`)
    Prp1,
}

/// A selection language keyword
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Keyword {
    // Logical operators
    Not,
    And,
    Or,
    In,
    Like,

    // Grouping operators
    ByRes,
    ByChain,
    ByObject,
    ByMolecule,
    BySegment,
    ByFragment,
    ByCAlpha,
    ByRing,
    ByCell,

    // Neighbor operators
    Neighbor,
    BoundTo,

    // Position operators
    First,
    Last,

    // Distance operators
    Around,
    Expand,
    Extend,
    Gap,
    Within,
    Beyond,
    NearTo,

    // Zero-argument selections
    All,
    None,
    Hetatm,
    Hydrogens,
    Visible,
    Enabled,
    Bonded,
    Polymer,
    PolymerProtein,
    PolymerNucleic,
    Organic,
    Inorganic,
    Solvent,
    Metals,
    Backbone,
    Sidechain,
    Donors,
    Acceptors,
    HBondAcceptors,
    HBondDonors,
    Delocalized,
    Fixed,
    Restrained,
    Masked,
    Protected,
    Present,
    Guide,
    Origin,
    Center,

    // One-argument selections
    Name,
    Element,
    Resi,
    Resn,
    Chain,
    Segi,
    Alt,
    Flag,
    TextType,
    NumericType,
    Id,
    Index,
    Rank,
    Model,
    State,
    Instance,
    SecondaryStructure,
    Rep,
    Color,
    CartoonColor,
    RibbonColor,
    PepSeq,
    Custom,
    Label,
    Stereo,

    // Two-argument numeric selections
    BFactor,
    Occupancy,
    PartialCharge,
    FormalCharge,
    X,
    Y,
    Z,

    // Property access
    Property,

    // Selection reference
    Selection,
}

impl Keyword {
    /// Get the keyword type
    pub fn keyword_type(self) -> KeywordType {
        match self {
            // Logical operators
            Keyword::Not => KeywordType::Opr1,
            Keyword::And | Keyword::Or | Keyword::In | Keyword::Like => KeywordType::Opr2,

            // Grouping operators (unary prefix)
            Keyword::ByRes
            | Keyword::ByChain
            | Keyword::ByObject
            | Keyword::ByMolecule
            | Keyword::BySegment
            | Keyword::ByFragment
            | Keyword::ByCAlpha
            | Keyword::ByRing
            | Keyword::ByCell
            | Keyword::Neighbor
            | Keyword::BoundTo
            | Keyword::First
            | Keyword::Last => KeywordType::Opr1,

            // Distance operators
            Keyword::Around | Keyword::Expand | Keyword::Extend | Keyword::Gap => KeywordType::Prp1,
            Keyword::Within | Keyword::Beyond | Keyword::NearTo => KeywordType::Op22,

            // Zero-argument selections
            Keyword::All
            | Keyword::None
            | Keyword::Hetatm
            | Keyword::Hydrogens
            | Keyword::Visible
            | Keyword::Enabled
            | Keyword::Bonded
            | Keyword::Polymer
            | Keyword::PolymerProtein
            | Keyword::PolymerNucleic
            | Keyword::Organic
            | Keyword::Inorganic
            | Keyword::Solvent
            | Keyword::Metals
            | Keyword::Backbone
            | Keyword::Sidechain
            | Keyword::Donors
            | Keyword::Acceptors
            | Keyword::HBondAcceptors
            | Keyword::HBondDonors
            | Keyword::Delocalized
            | Keyword::Fixed
            | Keyword::Restrained
            | Keyword::Masked
            | Keyword::Protected
            | Keyword::Present
            | Keyword::Guide
            | Keyword::Origin
            | Keyword::Center => KeywordType::Sel0,

            // One-argument selections
            Keyword::Name
            | Keyword::Element
            | Keyword::Resi
            | Keyword::Resn
            | Keyword::Chain
            | Keyword::Segi
            | Keyword::Alt
            | Keyword::Flag
            | Keyword::TextType
            | Keyword::NumericType
            | Keyword::Id
            | Keyword::Index
            | Keyword::Rank
            | Keyword::Model
            | Keyword::State
            | Keyword::Instance
            | Keyword::SecondaryStructure
            | Keyword::Rep
            | Keyword::Color
            | Keyword::CartoonColor
            | Keyword::RibbonColor
            | Keyword::PepSeq
            | Keyword::Custom
            | Keyword::Label
            | Keyword::Stereo
            | Keyword::Selection => KeywordType::Sel1,

            // Two-argument numeric selections
            Keyword::BFactor
            | Keyword::Occupancy
            | Keyword::PartialCharge
            | Keyword::FormalCharge
            | Keyword::X
            | Keyword::Y
            | Keyword::Z => KeywordType::Sel2,

            // Property access
            Keyword::Property => KeywordType::Sel3,
        }
    }

    /// Get the operator precedence (higher = binds tighter)
    pub fn precedence(self) -> u8 {
        match self {
            // Special selections - highest
            Keyword::All
            | Keyword::None
            | Keyword::Visible
            | Keyword::Enabled
            | Keyword::Hydrogens
            | Keyword::Origin
            | Keyword::Center => 0x90,

            // Property selections
            Keyword::Name
            | Keyword::Element
            | Keyword::Resi
            | Keyword::Resn
            | Keyword::Chain
            | Keyword::Segi
            | Keyword::Alt
            | Keyword::Flag
            | Keyword::TextType
            | Keyword::NumericType
            | Keyword::Id
            | Keyword::Index
            | Keyword::Rank
            | Keyword::Model
            | Keyword::State
            | Keyword::Instance
            | Keyword::SecondaryStructure
            | Keyword::Rep
            | Keyword::Color
            | Keyword::CartoonColor
            | Keyword::RibbonColor
            | Keyword::PepSeq
            | Keyword::Custom
            | Keyword::Label
            | Keyword::Stereo
            | Keyword::BFactor
            | Keyword::Occupancy
            | Keyword::PartialCharge
            | Keyword::FormalCharge
            | Keyword::X
            | Keyword::Y
            | Keyword::Z
            | Keyword::Property
            | Keyword::Hetatm
            | Keyword::Bonded
            | Keyword::Polymer
            | Keyword::PolymerProtein
            | Keyword::PolymerNucleic
            | Keyword::Organic
            | Keyword::Inorganic
            | Keyword::Solvent
            | Keyword::Metals
            | Keyword::Backbone
            | Keyword::Sidechain
            | Keyword::Donors
            | Keyword::Acceptors
            | Keyword::HBondAcceptors
            | Keyword::HBondDonors
            | Keyword::Delocalized
            | Keyword::Fixed
            | Keyword::Restrained
            | Keyword::Masked
            | Keyword::Protected
            | Keyword::Present
            | Keyword::Guide
            | Keyword::Selection => 0x80,

            // not
            Keyword::Not => 0x70,

            // and
            Keyword::And => 0x60,

            // bound_to
            Keyword::BoundTo => 0x50,

            // or, in, like
            Keyword::Or | Keyword::In | Keyword::Like => 0x40,

            // Distance operators
            Keyword::Around
            | Keyword::Expand
            | Keyword::Extend
            | Keyword::Gap
            | Keyword::Within
            | Keyword::Beyond
            | Keyword::NearTo
            | Keyword::First
            | Keyword::Last => 0x30,

            // Grouping operators
            Keyword::ByRes
            | Keyword::ByChain
            | Keyword::ByObject
            | Keyword::ByMolecule
            | Keyword::BySegment
            | Keyword::ByFragment
            | Keyword::ByCAlpha
            | Keyword::ByRing
            | Keyword::ByCell
            | Keyword::Neighbor => 0x20,
        }
    }
}

/// Static map of keyword strings to Keyword enum values
///
/// Includes all aliases (e.g., "n." for "name", "r." for "resn")
pub static KEYWORDS: phf::Map<&'static str, Keyword> = phf_map! {
    // Logical operators
    "not" => Keyword::Not,
    "!" => Keyword::Not,
    "and" => Keyword::And,
    "&" => Keyword::And,
    "or" => Keyword::Or,
    "|" => Keyword::Or,
    "+" => Keyword::Or,
    "-" => Keyword::And, // Special: AND NOT
    "in" => Keyword::In,
    "like" => Keyword::Like,
    "l." => Keyword::Like,

    // Grouping operators
    "byres" => Keyword::ByRes,
    "byresi" => Keyword::ByRes,
    "byresidue" => Keyword::ByRes,
    "br." => Keyword::ByRes,
    "bychain" => Keyword::ByChain,
    "bc." => Keyword::ByChain,
    "byobject" => Keyword::ByObject,
    "byobj" => Keyword::ByObject,
    "bo." => Keyword::ByObject,
    "bymolecule" => Keyword::ByMolecule,
    "bymol" => Keyword::ByMolecule,
    "bm." => Keyword::ByMolecule,
    "bysegment" => Keyword::BySegment,
    "byseg" => Keyword::BySegment,
    "bysegi" => Keyword::BySegment,
    "bs." => Keyword::BySegment,
    "byfragment" => Keyword::ByFragment,
    "byfrag" => Keyword::ByFragment,
    "bf." => Keyword::ByFragment,
    "bycalpha" => Keyword::ByCAlpha,
    "bca." => Keyword::ByCAlpha,
    "byring" => Keyword::ByRing,
    "bycell" => Keyword::ByCell,

    // Neighbor operators
    "neighbor" => Keyword::Neighbor,
    "nbr." => Keyword::Neighbor,
    "bound_to" => Keyword::BoundTo,
    "bto." => Keyword::BoundTo,

    // Position operators
    "first" => Keyword::First,
    "last" => Keyword::Last,

    // Distance operators
    "around" => Keyword::Around,
    "a." => Keyword::Around,
    "expand" => Keyword::Expand,
    "x." => Keyword::Expand,
    "extend" => Keyword::Extend,
    "xt." => Keyword::Extend,
    "gap" => Keyword::Gap,
    "within" => Keyword::Within,
    "w." => Keyword::Within,
    "beyond" => Keyword::Beyond,
    "be." => Keyword::Beyond,
    "near_to" => Keyword::NearTo,
    "nto." => Keyword::NearTo,

    // Zero-argument selections
    "all" => Keyword::All,
    "*" => Keyword::All,
    "none" => Keyword::None,
    "hetatm" => Keyword::Hetatm,
    "het" => Keyword::Hetatm,
    "hydrogens" => Keyword::Hydrogens,
    "hydro" => Keyword::Hydrogens,
    "h." => Keyword::Hydrogens,
    "visible" => Keyword::Visible,
    "v." => Keyword::Visible,
    "enabled" => Keyword::Enabled,
    "bonded" => Keyword::Bonded,
    "polymer" => Keyword::Polymer,
    "pol." => Keyword::Polymer,
    "polymer.protein" => Keyword::PolymerProtein,
    "protein" => Keyword::PolymerProtein,
    "pro." => Keyword::PolymerProtein,
    "polymer.nucleic" => Keyword::PolymerNucleic,
    "nucleic" => Keyword::PolymerNucleic,
    "nuc." => Keyword::PolymerNucleic,
    "organic" => Keyword::Organic,
    "org." => Keyword::Organic,
    "inorganic" => Keyword::Inorganic,
    "ino." => Keyword::Inorganic,
    "solvent" => Keyword::Solvent,
    "sol." => Keyword::Solvent,
    "metals" => Keyword::Metals,
    "backbone" => Keyword::Backbone,
    "bb." => Keyword::Backbone,
    "sidechain" => Keyword::Sidechain,
    "sc." => Keyword::Sidechain,
    "donors" => Keyword::Donors,
    "don." => Keyword::Donors,
    "acceptors" => Keyword::Acceptors,
    "acc." => Keyword::Acceptors,
    "hba." => Keyword::HBondAcceptors,
    "hbd." => Keyword::HBondDonors,
    "delocalized" => Keyword::Delocalized,
    "deloc." => Keyword::Delocalized,
    "fixed" => Keyword::Fixed,
    "fxd." => Keyword::Fixed,
    "restrained" => Keyword::Restrained,
    "rst." => Keyword::Restrained,
    "masked" => Keyword::Masked,
    "msk." => Keyword::Masked,
    "protected" => Keyword::Protected,
    "present" => Keyword::Present,
    "pr." => Keyword::Present,
    "guide" => Keyword::Guide,
    "origin" => Keyword::Origin,
    "center" => Keyword::Center,

    // One-argument selections
    "name" => Keyword::Name,
    "n." => Keyword::Name,
    "element" => Keyword::Element,
    "elem" => Keyword::Element,
    "symbol" => Keyword::Element,
    "e." => Keyword::Element,
    "resi" => Keyword::Resi,
    "resid" => Keyword::Resi,
    "residue" => Keyword::Resi,
    "resident" => Keyword::Resi,
    "i." => Keyword::Resi,
    "resn" => Keyword::Resn,
    "resname" => Keyword::Resn,
    "r." => Keyword::Resn,
    "chain" => Keyword::Chain,
    "c." => Keyword::Chain,
    "segi" => Keyword::Segi,
    "segid" => Keyword::Segi,
    "segment" => Keyword::Segi,
    "s." => Keyword::Segi,
    "alt" => Keyword::Alt,
    "altloc" => Keyword::Alt,
    "flag" => Keyword::Flag,
    "f." => Keyword::Flag,
    "text_type" => Keyword::TextType,
    "tt." => Keyword::TextType,
    "numeric_type" => Keyword::NumericType,
    "nt." => Keyword::NumericType,
    "id" => Keyword::Id,
    "ID" => Keyword::Id,
    "index" => Keyword::Index,
    "idx." => Keyword::Index,
    "rank" => Keyword::Rank,
    "model" => Keyword::Model,
    "object" => Keyword::Model,
    "m." => Keyword::Model,
    "o." => Keyword::Model,
    "state" => Keyword::State,
    "instance" => Keyword::Instance,
    "ss" => Keyword::SecondaryStructure,
    "rep" => Keyword::Rep,
    "color" => Keyword::Color,
    "cartoon_color" => Keyword::CartoonColor,
    "ribbon_color" => Keyword::RibbonColor,
    "pepseq" => Keyword::PepSeq,
    "ps." => Keyword::PepSeq,
    "custom" => Keyword::Custom,
    "label" => Keyword::Label,
    "stereo" => Keyword::Stereo,

    // Two-argument numeric selections
    "b" => Keyword::BFactor,
    "q" => Keyword::Occupancy,
    "partial_charge" => Keyword::PartialCharge,
    "pc." => Keyword::PartialCharge,
    "formal_charge" => Keyword::FormalCharge,
    "fc." => Keyword::FormalCharge,
    "x" => Keyword::X,
    "y" => Keyword::Y,
    "z" => Keyword::Z,

    // Property access
    "p." => Keyword::Property,

    // Selection reference
    "%" => Keyword::Selection,
};

/// Look up a keyword by name (case-insensitive)
pub fn lookup(name: &str) -> Option<Keyword> {
    // Try exact match first
    if let Some(&kw) = KEYWORDS.get(name) {
        return Some(kw);
    }
    // Try lowercase
    let lower = name.to_lowercase();
    KEYWORDS.get(lower.as_str()).copied()
}

/// Check if a name is a reserved keyword
#[allow(dead_code)]
pub fn is_keyword(name: &str) -> bool {
    lookup(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_lookup() {
        assert_eq!(lookup("name"), Some(Keyword::Name));
        assert_eq!(lookup("n."), Some(Keyword::Name));
        assert_eq!(lookup("NAME"), Some(Keyword::Name));
        assert_eq!(lookup("resn"), Some(Keyword::Resn));
        assert_eq!(lookup("r."), Some(Keyword::Resn));
        assert_eq!(lookup("all"), Some(Keyword::All));
        assert_eq!(lookup("*"), Some(Keyword::All));
    }

    #[test]
    fn test_keyword_type() {
        assert_eq!(Keyword::All.keyword_type(), KeywordType::Sel0);
        assert_eq!(Keyword::Name.keyword_type(), KeywordType::Sel1);
        assert_eq!(Keyword::BFactor.keyword_type(), KeywordType::Sel2);
        assert_eq!(Keyword::Not.keyword_type(), KeywordType::Opr1);
        assert_eq!(Keyword::And.keyword_type(), KeywordType::Opr2);
        assert_eq!(Keyword::Within.keyword_type(), KeywordType::Op22);
        assert_eq!(Keyword::Around.keyword_type(), KeywordType::Prp1);
    }

    #[test]
    fn test_precedence() {
        assert!(Keyword::All.precedence() > Keyword::Not.precedence());
        assert!(Keyword::Not.precedence() > Keyword::And.precedence());
        assert!(Keyword::And.precedence() > Keyword::Or.precedence());
        assert!(Keyword::Or.precedence() > Keyword::ByRes.precedence());
    }

    #[test]
    fn test_is_keyword() {
        assert!(is_keyword("name"));
        assert!(is_keyword("all"));
        assert!(is_keyword("byres"));
        assert!(!is_keyword("foobar"));
        assert!(!is_keyword("CA"));
    }
}
