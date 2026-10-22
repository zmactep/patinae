//! Coloring schemes and palette types for molecular visualization.

use serde::{Deserialize, Serialize};

use crate::constants::*;
use crate::Color;

/// Stable ID and accepted names of a built-in coloring scheme.
pub struct SchemeDescriptor {
    pub id: u32,
    /// Canonical name first, followed by aliases.
    pub names: &'static [&'static str],
}

macro_rules! define_schemes {
    ($( $constant:ident = $id:literal => [$($name:literal),+]; )+) => {
        /// Stable built-in scheme identifiers.
        pub mod ids { $(pub const $constant: u32 = $id;)+ }
        impl crate::ColorValue { $(pub const $constant: Self = Self::scheme($id);)+ }
        /// Built-in schemes used by parsing and inspection.
        pub const SCHEMES: &[SchemeDescriptor] = &[$(SchemeDescriptor { id: $id, names: &[$($name),+] }),+];
        /// Accepted names used by command completion.
        pub const SCHEME_NAMES: &[&str] = &[$($($name),+),+];
    };
}

define_schemes! {
    ELEMENT = 1 => ["element", "atomic", "cpk", "by_element"];
    CHAIN = 2 => ["chain", "by_chain", "chainbow"];
    SECONDARY_STRUCTURE = 3 => ["ss", "secondary_structure", "by_ss", "dssp"];
    B_FACTOR = 4 => ["b_factor", "b", "bfactor", "by_b"];
    RESIDUE_TYPE = 5 => ["residue_type", "residue", "by_residue", "aa_type"];
    RESIDUE_INDEX = 6 => ["residue_index", "index", "spectrum", "rainbow"];
}

// ---------------------------------------------------------------------------
// ElementPalette
// ---------------------------------------------------------------------------

/// Element-based coloring (CPK colors), indexed by atomic number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementPalette {
    #[serde(
        serialize_with = "serialize_color_array",
        deserialize_with = "deserialize_color_array"
    )]
    colors: [Color; 119],
}

fn serialize_color_array<S: serde::Serializer>(
    colors: &[Color; 119],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = serializer.serialize_seq(Some(119))?;
    for c in colors.iter() {
        seq.serialize_element(c)?;
    }
    seq.end()
}

fn deserialize_color_array<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<[Color; 119], D::Error> {
    let v: Vec<Color> = Vec::deserialize(deserializer)?;
    v.try_into().map_err(|v: Vec<Color>| {
        serde::de::Error::custom(format!("expected 119 colors, got {}", v.len()))
    })
}

impl ElementPalette {
    pub fn new() -> Self {
        let mut colors = [ELEM_UNKNOWN; 119];
        colors[0] = ELEM_UNKNOWN;
        colors[1] = ELEM_H;
        colors[2] = ELEM_HE;
        colors[3] = ELEM_LI;
        colors[4] = ELEM_BE;
        colors[5] = ELEM_B;
        colors[6] = ELEM_C;
        colors[7] = ELEM_N;
        colors[8] = ELEM_O;
        colors[9] = ELEM_F;
        colors[10] = ELEM_NE;
        colors[11] = ELEM_NA;
        colors[12] = ELEM_MG;
        colors[13] = ELEM_AL;
        colors[14] = ELEM_SI;
        colors[15] = ELEM_P;
        colors[16] = ELEM_S;
        colors[17] = ELEM_CL;
        colors[18] = ELEM_AR;
        colors[19] = ELEM_K;
        colors[20] = ELEM_CA;
        colors[26] = ELEM_FE;
        colors[29] = ELEM_CU;
        colors[30] = ELEM_ZN;
        colors[35] = ELEM_BR;
        colors[53] = ELEM_I;
        ElementPalette { colors }
    }

    pub fn get(&self, atomic_number: u8) -> Color {
        self.colors
            .get(atomic_number as usize)
            .copied()
            .unwrap_or(Color::MAGENTA)
    }

    pub fn get_by_symbol(&self, symbol: &str) -> Color {
        let atomic_number = match symbol.to_uppercase().as_str() {
            "H" => 1,
            "HE" => 2,
            "LI" => 3,
            "BE" => 4,
            "B" => 5,
            "C" => 6,
            "N" => 7,
            "O" => 8,
            "F" => 9,
            "NE" => 10,
            "NA" => 11,
            "MG" => 12,
            "AL" => 13,
            "SI" => 14,
            "P" => 15,
            "S" => 16,
            "CL" => 17,
            "AR" => 18,
            "K" => 19,
            "CA" => 20,
            "FE" => 26,
            "CU" => 29,
            "ZN" => 30,
            "BR" => 35,
            "I" => 53,
            _ => 0,
        };
        self.get(atomic_number)
    }

    pub fn set(&mut self, atomic_number: u8, color: Color) {
        if (atomic_number as usize) < self.colors.len() {
            self.colors[atomic_number as usize] = color;
        }
    }
}

impl Default for ElementPalette {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ChainPalette
// ---------------------------------------------------------------------------

/// Chain color palette — variable-length, wraps around.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainPalette {
    colors: Vec<Color>,
}

impl ChainPalette {
    pub fn new(colors: Vec<Color>) -> Self {
        assert!(
            !colors.is_empty(),
            "ChainPalette must have at least one color"
        );
        ChainPalette { colors }
    }

    /// Get color for a chain ID. Maps A-Z → 0-25, 0-9 → 26-35, wraps.
    pub fn get(&self, chain_id: &str) -> Color {
        let ch = chain_id.bytes().next().unwrap_or(b'A');
        let idx = if ch.is_ascii_uppercase() {
            (ch - b'A') as usize
        } else if ch.is_ascii_digit() {
            (ch - b'0') as usize + 26
        } else {
            0
        };
        self.colors[idx % self.colors.len()]
    }

    pub fn len(&self) -> usize {
        self.colors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    pub fn colors(&self) -> &[Color] {
        &self.colors
    }
}

// ---------------------------------------------------------------------------
// SsPalette
// ---------------------------------------------------------------------------

/// Secondary structure color palette.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SsPalette {
    pub helix: Color,
    pub sheet: Color,
    pub coil: Color,
    pub nucleic: Color,
}

impl SsPalette {
    pub fn new() -> Self {
        SsPalette {
            helix: SS_HELIX,
            sheet: SS_SHEET,
            coil: SS_LOOP,
            nucleic: SS_NUCLEIC,
        }
    }

    /// Get color by DSS ss_type code.
    pub fn get(&self, ss_type: u8) -> Color {
        match ss_type {
            1 | 3 | 4 => self.helix,
            2 => self.sheet,
            7 => self.nucleic,
            _ => self.coil,
        }
    }
}

impl Default for SsPalette {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ResiduePalette
// ---------------------------------------------------------------------------

/// Amino acid residue type color palette.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResiduePalette {
    pub hydrophobic: Color,
    pub positive: Color,
    pub negative: Color,
    pub polar: Color,
    pub cysteine: Color,
    pub glycine: Color,
}

impl ResiduePalette {
    pub fn new() -> Self {
        ResiduePalette {
            hydrophobic: RES_HYDROPHOBIC,
            positive: RES_POSITIVE,
            negative: RES_NEGATIVE,
            polar: RES_POLAR,
            cysteine: RES_CYSTEINE,
            glycine: RES_GLYCINE,
        }
    }

    /// Get color by 3-letter residue name. Returns None for non-AA.
    pub fn get(&self, resn: &str) -> Option<Color> {
        match resn {
            "ALA" | "VAL" | "LEU" | "ILE" | "PRO" | "PHE" | "MET" | "TRP" => Some(self.hydrophobic),
            "LYS" | "ARG" | "HIS" | "HID" | "HIE" | "HIP" => Some(self.positive),
            "ASP" | "GLU" => Some(self.negative),
            "SER" | "THR" | "ASN" | "GLN" => Some(self.polar),
            "CYS" | "CYX" => Some(self.cysteine),
            "GLY" => Some(self.glycine),
            _ => None,
        }
    }
}

impl Default for ResiduePalette {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_element_colors() {
        let colors = ElementPalette::new();
        let carbon = colors.get_by_symbol("C");
        assert!(carbon.r < 0.6 && carbon.g < 0.6 && carbon.b < 0.6);
        let nitrogen = colors.get_by_symbol("N");
        assert!(nitrogen.b > nitrogen.r && nitrogen.b > nitrogen.g);
        let oxygen = colors.get_by_symbol("O");
        assert!(oxygen.r > oxygen.g && oxygen.r > oxygen.b);
    }

    #[test]
    fn test_chain_palette_wraps() {
        let p = ChainPalette::new(vec![Color::RED, Color::GREEN, Color::BLUE]);
        let a = p.get("A");
        let d = p.get("D"); // wraps: 3 % 3 = 0
        assert_eq!(a, d);
    }

    #[test]
    fn test_chain_palette_digits() {
        let p = ChainPalette::new(CHAIN_DARK.to_vec());
        let c0 = p.get("0"); // 26 % 6 = 2
        assert_eq!(c0, CHAIN_DARK[2]);
    }

    #[test]
    fn test_ss_palette() {
        let ss = SsPalette::new();
        assert_eq!(ss.get(1), SS_HELIX);
        assert_eq!(ss.get(2), SS_SHEET);
        assert_eq!(ss.get(0), SS_LOOP);
        assert_eq!(ss.get(7), SS_NUCLEIC);
    }

    #[test]
    fn test_residue_palette() {
        let rp = ResiduePalette::new();
        assert_eq!(rp.get("ALA"), Some(RES_HYDROPHOBIC));
        assert_eq!(rp.get("LYS"), Some(RES_POSITIVE));
        assert_eq!(rp.get("ASP"), Some(RES_NEGATIVE));
        assert_eq!(rp.get("SER"), Some(RES_POLAR));
        assert_eq!(rp.get("CYS"), Some(RES_CYSTEINE));
        assert_eq!(rp.get("GLY"), Some(RES_GLYCINE));
        assert_eq!(rp.get("HOH"), None);
    }
}
