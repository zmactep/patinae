//! Enum-based settings with named variants.
//!
//! The [`SettingEnum`] trait provides a uniform interface for settings that have
//! a fixed set of named values (e.g., shading mode, mouse selection mode).
//! Each variant has a display name for the `set`/`get` commands and a stable
//! `i32` value for serialization.

use crate::setting::SettingValue;

/// Trait for setting enums with named variants.
///
/// Provides conversion between display names, i32 wire values, and Rust enum
/// variants. Used by the descriptor registry for `set`/`get` command hints and
/// by the GUI for combo-box rendering.
///
/// # Example
///
/// ```ignore
/// #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// #[repr(i32)]
/// pub enum ShadingMode {
///     Classic = 0,
///     Skripkin = 1,
///     Full = 2,
/// }
/// // impl SettingEnum for ShadingMode { ... }
/// ```
pub trait SettingEnum: Sized + Copy + PartialEq {
    /// All variants in declaration order (for autocomplete and GUI combo boxes).
    fn variants() -> &'static [Self];

    /// Display name for the variant (lowercase, for `set`/`get` commands).
    fn name(&self) -> &'static str;

    /// Parse from a string alias (case-insensitive).
    fn from_name(s: &str) -> Option<Self>;

    /// Convert from the wire i32 value. Returns a default variant for unknown values.
    fn from_i32(v: i32) -> Self;

    /// Convert to the wire i32 value.
    fn to_i32(self) -> i32;

    /// Value hints for the descriptor registry: `&[("name", SettingValue::Int(i32))]`.
    fn value_hints() -> &'static [(&'static str, SettingValue)];
}

// ---------------------------------------------------------------------------
// Macro to implement SettingEnum for simple #[repr(i32)] enums
// ---------------------------------------------------------------------------

/// Implement [`SettingEnum`] for a `#[repr(i32)]` enum.
///
/// # Usage
///
/// ```ignore
/// impl_setting_enum! {
///     ShadingMode {
///         Classic = 0 => "classic",
///         Skripkin = 1 => "skripkin",
///         Full = 2 => "full",
///     }
///     default: Classic
/// }
/// ```
#[macro_export]
macro_rules! impl_setting_enum {
    (
        $Enum:ident {
            $( $Variant:ident = $val:expr => $name:expr ),+ $(,)?
        }
        default: $Default:ident
    ) => {
        impl $crate::SettingEnum for $Enum {
            fn variants() -> &'static [Self] {
                static VARIANTS: &[$Enum] = &[
                    $( $Enum::$Variant ),+
                ];
                VARIANTS
            }

            fn name(&self) -> &'static str {
                match self {
                    $( $Enum::$Variant => $name ),+
                }
            }

            fn from_name(s: &str) -> Option<Self> {
                let lower = s.to_lowercase();
                match lower.as_str() {
                    $( $name => Some($Enum::$Variant), )+
                    _ => None,
                }
            }

            fn from_i32(v: i32) -> Self {
                match v {
                    $( $val => $Enum::$Variant, )+
                    _ => $Enum::$Default,
                }
            }

            fn to_i32(self) -> i32 {
                self as i32
            }

            fn value_hints() -> &'static [(&'static str, $crate::SettingValue)] {
                static HINTS: &[(&str, $crate::SettingValue)] = &[
                    $( ($name, $crate::SettingValue::Int($val)) ),+
                ];
                HINTS
            }
        }

        impl From<i32> for $Enum {
            fn from(v: i32) -> Self {
                <$Enum as $crate::SettingEnum>::from_i32(v)
            }
        }

        impl From<$Enum> for i32 {
            fn from(v: $Enum) -> i32 {
                <$Enum as $crate::SettingEnum>::to_i32(v)
            }
        }
    };
}

// ---------------------------------------------------------------------------
// MouseSelectionMode enum
// ---------------------------------------------------------------------------

/// Mouse selection granularity mode.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[repr(i32)]
pub enum MouseSelectionMode {
    Atoms = 0,
    #[default]
    Residues = 1,
    Chains = 2,
    Segments = 3,
    Objects = 4,
    Molecules = 5,
    CAlphas = 6,
}

impl_setting_enum! {
    MouseSelectionMode {
        Atoms = 0 => "atoms",
        Residues = 1 => "residues",
        Chains = 2 => "chains",
        Segments = 3 => "segments",
        Objects = 4 => "objects",
        Molecules = 5 => "molecules",
        CAlphas = 6 => "c_alphas",
    }
    default: Residues
}

// ---------------------------------------------------------------------------
// DssAlgorithm enum
// ---------------------------------------------------------------------------

/// Algorithm for secondary structure assignment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[repr(i32)]
pub enum DssAlgorithm {
    /// PyMOL-derived DSS algorithm (H-bonds + phi/psi dihedral angles)
    #[default]
    PyMol = 0,
    /// DSSP algorithm (Kabsch & Sander)
    Dssp = 1,
    /// Dummy algorithm — assigns Loop to every residue
    None = 2,
}

impl_setting_enum! {
    DssAlgorithm {
        PyMol = 0 => "pymol",
        Dssp = 1 => "dssp",
        None = 2 => "none",
    }
    default: PyMol
}

// ---------------------------------------------------------------------------
// ThemeMode enum
// ---------------------------------------------------------------------------

/// Visual theme for the application.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[repr(i32)]
pub enum ThemeMode {
    Dark = 0,
    Light = 1,
    #[default]
    System = 2,
}

impl_setting_enum! {
    ThemeMode {
        Dark = 0 => "dark",
        Light = 1 => "light",
        System = 2 => "system",
    }
    default: System
}

impl ThemeMode {
    /// Returns the next preference in the sidebar button's cycle.
    pub fn next(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::System,
            Self::System => Self::Dark,
        }
    }
}

// ---------------------------------------------------------------------------
// RenderMemoryProfileSetting enum
// ---------------------------------------------------------------------------

/// Renderer memory profile selected by user settings.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[repr(i32)]
pub enum RenderMemoryProfileSetting {
    #[default]
    Auto = 0,
    Performance = 1,
    Balanced = 2,
    Lite = 3,
    Manual = 4,
}

impl_setting_enum! {
    RenderMemoryProfileSetting {
        Auto = 0 => "auto",
        Performance = 1 => "performance",
        Balanced = 2 => "balanced",
        Lite = 3 => "lite",
        Manual = 4 => "manual",
    }
    default: Auto
}

// ---------------------------------------------------------------------------
// SurfaceMode enum
// ---------------------------------------------------------------------------

/// Controls which atoms participate in surface generation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[repr(i32)]
pub enum SurfaceMode {
    /// Surface biopolymer atoms only (protein + nucleic).
    #[default]
    Normal = 0,
    /// Surface everything, including HET and hydrogens.
    All = 1,
    /// Surface only heavy atoms (no hydrogens).
    Heavy = 2,
    /// Surface only visible atoms.
    Visible = 3,
    /// Surface visible heavy atoms.
    VisibleHeavy = 4,
}

impl_setting_enum! {
    SurfaceMode {
        Normal = 0 => "normal",
        All = 1 => "all",
        Heavy = 2 => "heavy",
        Visible = 3 => "visible",
        VisibleHeavy = 4 => "visible_heavy",
    }
    default: Normal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mouse_selection_mode_roundtrip() {
        for &v in MouseSelectionMode::variants() {
            let name = v.name();
            let parsed = MouseSelectionMode::from_name(name).unwrap();
            assert_eq!(parsed, v);
            assert_eq!(MouseSelectionMode::from_i32(v.to_i32()), v);
        }
    }

    #[test]
    fn test_mouse_selection_mode_hints() {
        let hints = MouseSelectionMode::value_hints();
        assert_eq!(hints.len(), 7);
        assert_eq!(hints[0].0, "atoms");
    }

    #[test]
    fn test_unknown_i32_returns_default() {
        assert_eq!(
            MouseSelectionMode::from_i32(99),
            MouseSelectionMode::Residues
        );
    }

    #[test]
    fn test_case_insensitive_name() {
        assert_eq!(
            MouseSelectionMode::from_name("ATOMS"),
            Some(MouseSelectionMode::Atoms)
        );
        assert_eq!(
            MouseSelectionMode::from_name("atoms"),
            Some(MouseSelectionMode::Atoms)
        );
        assert_eq!(
            MouseSelectionMode::from_name("Residues"),
            Some(MouseSelectionMode::Residues)
        );
        assert!(MouseSelectionMode::from_name("unknown").is_none());
    }
}
