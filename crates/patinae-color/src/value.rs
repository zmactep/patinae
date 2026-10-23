//! Stable 32-bit color values independent of molecules, palettes, and rendering.
//!
//! The high byte is a tag; the low 24 bits are its payload. Tags 1, 2, 3,
//! and 4 denote RGB, palette indices, scheme IDs, and control states.
//! Tag 0 is invalid, and tags 5..=255 are reserved for
//! extensions. Unknown tagged values survive serialization without truncation.
//! RGB payloads use 0xRRGGBB, independently of GPU buffer byte order.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Largest palette index or scheme ID representable in a color value.
pub const MAX_COLOR_ID: u32 = 0x00ff_ffff;

/// Category of a color value; all data remains in ColorValue.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorKind {
    Rgb = 1,
    Palette = 2,
    Scheme = 3,
    Inherit = 4,
    /// Use the atom's base color without a global representation override.
    Base = 5,
    Unknown = 255,
}

/// One tagged color value occupying exactly four bytes.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorValue(u32);

impl ColorValue {
    pub const INHERIT: Self = Self(0x0400_0000);
    /// Follow the atom's base color, bypassing global representation defaults.
    pub const BASE: Self = Self(0x0400_0001);

    /// Parse a built-in scheme name or one of its aliases.
    pub fn from_scheme_name(name: &str) -> Option<Self> {
        crate::SCHEMES
            .iter()
            .find(|scheme| {
                scheme
                    .names
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(name))
            })
            .map(|scheme| Self::scheme(scheme.id))
    }

    /// Construct an RGB value without palette registration.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self(0x0100_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    /// Construct a palette reference, rejecting indices that exceed 24 bits.
    pub const fn try_palette(index: u32) -> Option<Self> {
        if index <= MAX_COLOR_ID {
            Some(Self(0x0200_0000 | index))
        } else {
            None
        }
    }

    /// Construct a palette reference from an internally allocated index.
    ///
    /// # Panics
    /// Panics if `index` exceeds [`MAX_COLOR_ID`]. Use `try_palette` for external input.
    pub const fn palette(index: u32) -> Self {
        assert!(index <= MAX_COLOR_ID, "palette index exceeds 24 bits");
        Self(0x0200_0000 | index)
    }

    /// Construct a scheme reference, rejecting IDs that exceed 24 bits.
    pub const fn try_scheme(id: u32) -> Option<Self> {
        if id <= MAX_COLOR_ID {
            Some(Self(0x0300_0000 | id))
        } else {
            None
        }
    }

    /// Construct a scheme reference from a known ID.
    ///
    /// # Panics
    /// Panics if `id` exceeds [`MAX_COLOR_ID`]. Use `try_scheme` for external input.
    pub const fn scheme(id: u32) -> Self {
        assert!(id <= MAX_COLOR_ID, "scheme ID exceeds 24 bits");
        Self(0x0300_0000 | id)
    }

    /// Read a tagged value without interpreting palette indices.
    ///
    /// Unknown tags and scheme IDs are preserved.
    ///
    /// # Errors
    /// Returns an error for tag zero, which has no encoding tag.
    pub const fn try_from_bits(bits: u32) -> Result<Self, &'static str> {
        if bits >> 24 == 0 {
            Err("color requires a nonzero encoding tag")
        } else {
            Ok(Self(bits))
        }
    }

    /// Construct a known tagged value.
    ///
    /// # Panics
    /// Panics for tag zero. Use try_from_bits for external data.
    pub const fn from_bits(bits: u32) -> Self {
        match Self::try_from_bits(bits) {
            Ok(value) => value,
            Err(_) => panic!("color requires a nonzero encoding tag"),
        }
    }

    /// Return the complete encoded value, including unknown tags.
    pub const fn to_bits(self) -> u32 {
        self.0
    }

    /// Read RGB components only from a literal RGB value.
    pub const fn as_rgb8(self) -> Option<[u8; 3]> {
        if self.0 >> 24 == 1 {
            Some([(self.0 >> 16) as u8, (self.0 >> 8) as u8, self.0 as u8])
        } else {
            None
        }
    }

    /// Read a palette index only from a palette reference.
    pub const fn palette_index(self) -> Option<u32> {
        if self.0 >> 24 == 2 {
            Some(self.0 & MAX_COLOR_ID)
        } else {
            None
        }
    }

    /// Read a scheme ID only from a scheme reference.
    pub const fn scheme_id(self) -> Option<u32> {
        if self.0 >> 24 == 3 {
            Some(self.0 & MAX_COLOR_ID)
        } else {
            None
        }
    }

    /// Inspect the category without copying or interpreting its data.
    pub const fn kind(self) -> ColorKind {
        match self.0 >> 24 {
            1 => ColorKind::Rgb,
            2 => ColorKind::Palette,
            3 => ColorKind::Scheme,
            4 if self.0 == Self::INHERIT.0 => ColorKind::Inherit,
            4 if self.0 == Self::BASE.0 => ColorKind::Base,
            _ => ColorKind::Unknown,
        }
    }
}

impl Default for ColorValue {
    fn default() -> Self {
        Self::INHERIT
    }
}

impl std::fmt::Display for ColorValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some([r, g, b]) = self.as_rgb8() {
            write!(f, "#{r:02x}{g:02x}{b:02x}")
        } else if let Some(id) = self.palette_index() {
            write!(f, "palette:{id}")
        } else if let Some(id) = self.scheme_id() {
            write!(f, "scheme:{id}")
        } else if *self == Self::INHERIT {
            f.write_str("inherit")
        } else if *self == Self::BASE {
            f.write_str("base")
        } else {
            write!(f, "color:0x{:08x}", self.0)
        }
    }
}

impl Serialize for ColorValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.0)
    }
}

impl std::str::FromStr for ColorValue {
    type Err = &'static str;

    /// Parse the canonical display form or a built-in scheme name.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let text = text.trim();
        if text == "inherit" {
            return Ok(Self::INHERIT);
        }
        if text == "base" {
            return Ok(Self::BASE);
        }
        if let Some(id) = text.strip_prefix("palette:") {
            return id
                .parse::<u32>()
                .ok()
                .and_then(Self::try_palette)
                .ok_or("invalid palette index");
        }
        if let Some(id) = text.strip_prefix("scheme:") {
            return id
                .parse::<u32>()
                .ok()
                .and_then(Self::try_scheme)
                .ok_or("invalid scheme ID");
        }
        if let Some(bits) = text.strip_prefix("color:0x") {
            return Self::try_from_bits(
                u32::from_str_radix(bits, 16).map_err(|_| "invalid encoded color")?,
            );
        }
        if let Some(rgb) = text.strip_prefix('#').or_else(|| text.strip_prefix("0x")) {
            if rgb.len() != 6 {
                return Err("RGB requires six hexadecimal digits");
            }
            let rgb = u32::from_str_radix(rgb, 16).map_err(|_| "invalid RGB color")?;
            return Ok(Self::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8));
        }
        if let Ok(index) = text.parse::<u32>() {
            return Self::try_palette(index).ok_or("invalid palette index");
        }
        Self::from_scheme_name(text).ok_or("unknown color value")
    }
}

impl<'de> Deserialize<'de> for ColorValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from_bits(u32::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_and_namespaces_are_disjoint() {
        assert_eq!(std::mem::size_of::<ColorValue>(), 4);
        for payload in [0, 1, 255, 65535, MAX_COLOR_ID] {
            let rgb = ColorValue::rgb((payload >> 16) as u8, (payload >> 8) as u8, payload as u8);
            assert_ne!(rgb, ColorValue::palette(payload));
            assert_ne!(rgb, ColorValue::scheme(payload));
            assert_ne!(ColorValue::palette(payload), ColorValue::scheme(payload));
            assert_eq!(ColorValue::palette(payload).kind(), ColorKind::Palette);
            assert_eq!(ColorValue::scheme(payload).kind(), ColorKind::Scheme);
        }
        assert!(ColorValue::try_palette(MAX_COLOR_ID + 1).is_none());
        assert!(ColorValue::try_scheme(MAX_COLOR_ID + 1).is_none());
    }

    #[test]
    fn tagged_values_preserve_all_bits() {
        assert_eq!(std::mem::size_of::<ColorKind>(), 1);
        for bits in [
            0x01000000, 0x01ffffff, 0x02ffffff, 0x03ffffff, 0x04000000, 0x04000001, 0xfe123456,
            0xffffffff, 0x80000000,
        ] {
            let value = ColorValue::try_from_bits(bits).unwrap();
            assert_eq!(value.to_bits(), bits);
            assert_eq!(
                ColorValue::try_from_bits((bits as i32) as u32).unwrap(),
                value
            );
            assert_eq!(value.to_string().parse::<ColorValue>().unwrap(), value);
        }
        assert!(ColorValue::try_from_bits(2).is_err());
        assert!(ColorValue::try_from_bits(0xffffff).is_err());
        assert_eq!(ColorValue::rgb(1, 2, 3).palette_index(), None);
        assert_eq!(ColorValue::palette(2).scheme_id(), None);
        assert_eq!(ColorValue::scheme(2).as_rgb8(), None);
    }

    #[test]
    fn scheme_catalog_is_unique_and_drives_parsing_and_completion() {
        let mut ids = std::collections::HashSet::new();
        let mut names = std::collections::HashSet::new();
        for scheme in crate::SCHEMES {
            assert!(ids.insert(scheme.id));
            for name in scheme.names {
                assert!(names.insert(*name));
                assert!(crate::SCHEME_NAMES.contains(name));
                assert_eq!(
                    name.to_uppercase()
                        .parse::<ColorValue>()
                        .unwrap()
                        .scheme_id(),
                    Some(scheme.id)
                );
            }
        }
        assert_eq!(names.len(), crate::SCHEME_NAMES.len());
        assert!("scheme:16777216".parse::<ColorValue>().is_err());
        assert!("palette:16777216".parse::<ColorValue>().is_err());
        assert!("color:0x00000002".parse::<ColorValue>().is_err());
    }
}
