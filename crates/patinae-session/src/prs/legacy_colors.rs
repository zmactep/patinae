//! PRS v1-v6 color migration, kept outside the current tagged color contract.

use std::collections::HashMap;

use patinae_color::{ColorValue, NamedPalette};
use rmpv::Value;

use super::{invalid, session_value, session_value_mut, PrsError};

const COLOR_FIELDS: &[&str] = &[
    "base",
    "cartoon",
    "ribbon",
    "stick",
    "line",
    "surface",
    "mesh",
    "sphere",
    "dot",
    "ellipsoid",
];

// The v6 positions are part of the file format, not the current settings layout.
const GROUPS: &[(&str, usize, usize, usize)] = &[
    ("cartoon", 1, 14, 23),
    ("stick", 2, 1, 5),
    ("sphere", 3, 1, 3),
    ("surface", 4, 3, 10),
    ("ribbon", 5, 5, 6),
    ("line", 6, 0, 1),
    ("dot", 7, 0, 3),
    ("mesh", 8, 0, 5),
    ("ellipsoid", 9, 2, 4),
];

type Defaults = HashMap<String, ColorValue>;

pub(super) fn migrate(root: &mut Value) -> Result<(), PrsError> {
    let (version, session) = session_value(root)?;
    if version.is_some_and(|v| v > 6) || !has_legacy_colors(session) {
        return Ok(());
    }
    let session = session_value_mut(root)?;
    let palette = field(session, "named_palette", 7)
        .map(decode::<NamedPalette>)
        .transpose()?
        .unwrap_or_default();
    let mut defaults = Defaults::new();
    if let Some(settings) = field_mut(session, "settings", 6) {
        defaults = migrate_settings(settings, &palette)?;
    }
    if let Some(registry) = field_mut(session, "registry", 0) {
        for (name, index, state_index, overrides_index) in [
            ("molecules", 0, 1, Some(3)),
            ("groups", 1, 1, None),
            ("maps", 2, 1, Some(8)),
            ("measurements", 9, 2, None),
            ("labels", 10, 0, None),
        ] {
            if let Some(Value::Array(owners)) = field_mut(registry, name, index) {
                for owner in owners {
                    let Value::Array(pair) = owner else { continue };
                    if pair.len() != 2 {
                        continue;
                    }
                    let snapshot = &mut pair[1];
                    migrate_state(field_mut(snapshot, "state", state_index))?;
                    let mut local = Defaults::new();
                    if let Some(index) = overrides_index {
                        if let Some(overrides) = field_mut(snapshot, "overrides", index) {
                            local = migrate_overrides(overrides, &palette)?;
                        }
                    }
                    if name == "molecules" {
                        if let Some(molecule) = field_mut(snapshot, "molecule", 0) {
                            migrate_dynamic_settings(molecule, &palette)?;
                            if let Some(Value::Array(atoms)) = field_mut(molecule, "atoms", 0) {
                                for atom in atoms {
                                    let nucleic = field(atom, "state", 19)
                                        .and_then(|state| field(state, "flags", 0))
                                        .and_then(Value::as_u64)
                                        .is_some_and(|flags| {
                                            flags
                                                & u64::from(patinae_mol::AtomFlags::NUCLEIC.bits())
                                                != 0
                                        });
                                    if let Some(colors) = field_mut(atom, "repr", 20)
                                        .and_then(|repr| field_mut(repr, "colors", 0))
                                    {
                                        migrate_atom_colors(colors, &local, &defaults, nucleic)?;
                                    }
                                }
                            }
                        }
                    } else if name == "measurements" || name == "labels" {
                        let measurement = name == "measurements";
                        migrate_presentation(field_mut(
                            snapshot,
                            "presentation",
                            if measurement { 5 } else { 2 },
                        ))?;
                        if let Some(Value::Array(entities)) = field_mut(
                            snapshot,
                            if measurement { "entries" } else { "entities" },
                            1,
                        ) {
                            for entity in entities {
                                migrate_presentation(field_mut(
                                    entity,
                                    "presentation",
                                    if measurement { 1 } else { 2 },
                                ))?;
                            }
                        }
                    }
                }
            }
        }
        if let Some(Value::Array(states)) = field_mut(registry, "object_states", 4) {
            for pair in states {
                if let Value::Array(pair) = pair {
                    migrate_state(pair.get_mut(1))?;
                }
            }
        }
    }
    if let Some(manager) = field_mut(session, "scenes", 3) {
        if let Some(Value::Map(scenes)) = field_mut(manager, "scenes", 0) {
            for (_, scene) in scenes {
                // v6 carried an unused unique-id map before object_data.
                remove_field(scene, "atom_data", 6, 8);
                if let Some(Value::Map(objects)) = field_mut(scene, "object_data", 6) {
                    for (_, object) in objects {
                        migrate_state(Some(object))?;
                        if let Some(Value::Array(atoms)) = field_mut(object, "per_atom_data", 6) {
                            for atom in atoms.iter_mut() {
                                let old_color = field(atom, "color", 0).cloned();
                                if let Some(old_color) = old_color.filter(is_legacy_color) {
                                    let colors = Value::Map(vec![(
                                        Value::from("base"),
                                        Value::from(atom_color(&old_color)?.to_bits()),
                                    )]);
                                    match atom {
                                        Value::Map(fields) => {
                                            fields.retain(|(key, _)| key.as_str() != Some("color"));
                                            fields.push((Value::from("colors"), colors));
                                            fields.push((
                                                Value::from("base_color_only"),
                                                Value::from(true),
                                            ));
                                        }
                                        Value::Array(fields) => {
                                            fields[0] = colors;
                                            fields.push(Value::from(true));
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn has_legacy_colors(session: &Value) -> bool {
    if let Some(settings) = field(session, "settings", 6) {
        if GROUPS.iter().any(|&(name, index, color_index, _)| {
            field(settings, name, index + 8)
                .filter(|g| matches!(g, Value::Map(_)))
                .and_then(|g| field(g, "color", color_index))
                .is_some_and(is_legacy_color)
        }) {
            return true;
        }
    }
    let setting = field(session, "settings", 6)
        .and_then(|settings| field(settings, "shading", 0))
        .and_then(|shading| field(shading, "common", 1))
        .and_then(|common| field(common, "silhouette_color", 2));
    if setting.is_some_and(is_legacy_color) {
        return true;
    }
    let registry = field(session, "registry", 0);
    if let Some(Value::Array(states)) = registry.and_then(|r| field(r, "object_states", 4)) {
        if states.iter().any(|pair| {
            pair.as_array()
                .and_then(|p| p.get(1))
                .and_then(|s| field(s, "color", 1))
                .is_some_and(is_legacy_color)
        }) {
            return true;
        }
    }
    if let Some(Value::Array(molecules)) = registry.and_then(|r| field(r, "molecules", 0)) {
        return molecules.iter().any(|pair| {
            pair.as_array()
                .and_then(|p| p.get(1))
                .and_then(|s| field(s, "molecule", 0))
                .and_then(|m| field(m, "atoms", 0))
                .and_then(Value::as_array)
                .is_some_and(|atoms| {
                    atoms.iter().any(|atom| {
                        field(atom, "repr", 20)
                            .and_then(|r| field(r, "colors", 0))
                            .and_then(|c| field(c, "base", 0))
                            .is_some_and(is_legacy_color)
                    })
                })
        });
    }
    false
}

fn is_legacy_color(value: &Value) -> bool {
    match value {
        Value::Integer(_) => value.as_u64().is_none_or(|v| v < 0x0100_0000),
        Value::String(_) | Value::Map(_) => true,
        _ => false,
    }
}

fn field<'a>(value: &'a Value, name: &str, index: usize) -> Option<&'a Value> {
    match value {
        Value::Map(fields) => fields
            .iter()
            .find(|(key, _)| key.as_str() == Some(name))
            .map(|(_, v)| v),
        Value::Array(fields) => fields.get(index),
        _ => None,
    }
}

fn field_mut<'a>(value: &'a mut Value, name: &str, index: usize) -> Option<&'a mut Value> {
    match value {
        Value::Map(fields) => fields
            .iter_mut()
            .find(|(key, _)| key.as_str() == Some(name))
            .map(|(_, v)| v),
        Value::Array(fields) => fields.get_mut(index),
        _ => None,
    }
}

fn remove_field(value: &mut Value, name: &str, index: usize, old_len: usize) -> Option<Value> {
    match value {
        Value::Map(fields) => fields
            .iter()
            .position(|(key, _)| key.as_str() == Some(name))
            .map(|i| fields.remove(i).1),
        Value::Array(fields) if fields.len() == old_len => Some(fields.remove(index)),
        _ => None,
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, PrsError> {
    let mut bytes = Vec::new();
    rmpv::encode::write_value(&mut bytes, value).map_err(|error| invalid(error.to_string()))?;
    Ok(rmp_serde::from_slice(&bytes)?)
}

fn atom_color(value: &Value) -> Result<ColorValue, PrsError> {
    if let Some(value) = value.as_i64() {
        return Ok(match value {
            -1 => ColorValue::ELEMENT,
            -2 => ColorValue::CHAIN,
            -3 => ColorValue::SECONDARY_STRUCTURE,
            -4 => ColorValue::B_FACTOR,
            -5 => ColorValue::RESIDUE_TYPE,
            -6 => ColorValue::RESIDUE_INDEX,
            v if v == i64::from(i32::MIN) => ColorValue::INHERIT,
            0..=0x00ff_ffff => ColorValue::palette(value as u32),
            0x0100_0000..=0xffff_ffff => ColorValue::from_bits(value as u32),
            _ => return Err(invalid(format!("invalid legacy atom color {value}"))),
        });
    }
    // ColorIndex was a serde enum: unit variants are strings, Named is a map.
    if let Some(name) = value.as_str() {
        return match name {
            "ByElement" | "Atomic" => Ok(ColorValue::ELEMENT),
            "ByChain" => Ok(ColorValue::CHAIN),
            "BySS" => Ok(ColorValue::SECONDARY_STRUCTURE),
            "ByBFactor" => Ok(ColorValue::B_FACTOR),
            "ByResidueType" => Ok(ColorValue::RESIDUE_TYPE),
            "ByResidueIndex" => Ok(ColorValue::RESIDUE_INDEX),
            _ => Err(invalid(format!("unknown legacy color variant {name}"))),
        };
    }
    if let Value::Map(fields) = value {
        if fields.len() == 1 && fields[0].0.as_str() == Some("Named") {
            return fields[0]
                .1
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .and_then(ColorValue::try_palette)
                .ok_or_else(|| invalid("invalid legacy palette index"));
        }
    }
    Err(invalid("invalid legacy color"))
}

fn setting_color(value: &Value, palette: &NamedPalette) -> Result<ColorValue, PrsError> {
    if !is_legacy_color(value) {
        return atom_color(value);
    }
    if value.as_i64().is_some_and(|v| v < 0) {
        return Ok(ColorValue::INHERIT);
    }
    let color = atom_color(value)?;
    // Legacy setting colors used palette lookup first, then packed RGB.
    if let Some(index) = color.palette_index() {
        if palette.get_by_index(index).is_none() {
            return Ok(ColorValue::rgb(
                (index >> 16) as u8,
                (index >> 8) as u8,
                index as u8,
            ));
        }
    }
    Ok(color)
}

fn migrate_state(state: Option<&mut Value>) -> Result<(), PrsError> {
    if let Some(color) = state.and_then(|s| field_mut(s, "color", 1)) {
        *color = Value::from(atom_color(color)?.to_bits());
    }
    Ok(())
}

fn migrate_presentation(presentation: Option<&mut Value>) -> Result<(), PrsError> {
    if let Some(color) = presentation.and_then(|p| field_mut(p, "color", 0)) {
        if !color.is_nil() {
            *color = Value::from(atom_color(color)?.to_bits());
        }
    }
    Ok(())
}

fn migrate_settings(settings: &mut Value, palette: &NamedPalette) -> Result<Defaults, PrsError> {
    let mut defaults = Defaults::new();
    for &(name, index, color_index, old_len) in GROUPS {
        if let Some(group) = field_mut(settings, name, index + 8) {
            let count = if name == "cartoon" { 2 } else { 1 };
            let mut moved = Vec::new();
            for offset in (0..count).rev() {
                let key = if offset == 0 {
                    "color"
                } else {
                    "nucleic_acid_color"
                };
                if let Some(color) = field_mut(group, key, color_index + offset) {
                    let converted = setting_color(color, palette)?;
                    defaults.insert(
                        if offset == 0 {
                            name.to_owned()
                        } else {
                            "cartoon_nucleic".to_owned()
                        },
                        converted,
                    );
                    *color = Value::from(converted.to_bits());
                }
                if let Value::Array(fields) = group {
                    if fields.len() == old_len - moved.len() {
                        moved.push(fields.remove(color_index + offset));
                    }
                }
            }
            if let Value::Array(fields) = group {
                fields.extend(moved.into_iter().rev());
            }
        }
    }
    if let Some(common) = field_mut(settings, "shading", 0).and_then(|s| field_mut(s, "common", 1))
    {
        if let Some(color) = field_mut(common, "silhouette_color", 2) {
            *color = Value::from(setting_color(color, palette)?.to_bits());
        }
    }
    if let Some(measurement) = field_mut(settings, "measurement", 7) {
        for (name, index) in [
            ("dash_color", 12),
            ("angle_color", 13),
            ("dihedral_color", 14),
        ] {
            if let Some(color) = field_mut(measurement, name, index) {
                *color = Value::from(setting_color(color, palette)?.to_bits());
            }
        }
    }
    Ok(defaults)
}

fn migrate_overrides(overrides: &mut Value, palette: &NamedPalette) -> Result<Defaults, PrsError> {
    let mut colors = Defaults::new();
    for &(name, index, color_index, old_len) in GROUPS {
        if let Some(group) = field_mut(overrides, name, index) {
            if name == "cartoon" {
                if let Some(color) =
                    remove_field(group, "nucleic_acid_color", color_index + 1, old_len)
                {
                    if !color.is_nil() {
                        colors.insert(
                            "cartoon_nucleic".to_owned(),
                            setting_color(&color, palette)?,
                        );
                    }
                }
            }
            if let Some(color) = remove_field(
                group,
                "color",
                color_index,
                old_len - usize::from(name == "cartoon"),
            ) {
                if !color.is_nil() {
                    colors.insert(name.to_owned(), setting_color(&color, palette)?);
                }
            }
        }
    }
    Ok(colors)
}

fn default_atom_colors() -> Value {
    Value::Map(
        COLOR_FIELDS
            .iter()
            .enumerate()
            .map(|(index, name)| {
                (
                    Value::from(*name),
                    Value::from(match index {
                        0 => -1,
                        1 => -2,
                        _ => i32::MIN,
                    }),
                )
            })
            .collect(),
    )
}

fn migrate_atom_colors(
    colors: &mut Value,
    local: &Defaults,
    global: &Defaults,
    nucleic: bool,
) -> Result<(), PrsError> {
    // AtomColors used serde(default), whose base/cartoon defaults changed in v8.
    if let Value::Map(fields) = colors {
        let Value::Map(defaults) = default_atom_colors() else {
            unreachable!()
        };
        for (key, value) in defaults {
            if !fields.iter().any(|(existing, _)| existing == &key) {
                fields.push((key, value));
            }
        }
    } else if let Value::Array(fields) = colors {
        for index in fields.len()..COLOR_FIELDS.len() {
            fields.push(Value::from(match index {
                0 => -1,
                1 => -2,
                _ => i32::MIN,
            }));
        }
    }
    for (index, &name) in COLOR_FIELDS.iter().enumerate() {
        if let Some(value) = field_mut(colors, name, index) {
            let mut color = atom_color(value)?;
            if index != 0 {
                let fallback = local
                    .get(name)
                    .or_else(|| global.get(name))
                    .copied()
                    .unwrap_or(ColorValue::INHERIT);
                if name == "cartoon" && (color == ColorValue::CHAIN || color == ColorValue::INHERIT)
                {
                    let nucleic_default = local
                        .get("cartoon_nucleic")
                        .or_else(|| global.get("cartoon_nucleic"))
                        .copied()
                        .unwrap_or(ColorValue::INHERIT);
                    if nucleic && nucleic_default != ColorValue::INHERIT {
                        color = nucleic_default;
                    } else if fallback != ColorValue::INHERIT {
                        color = fallback;
                    } else if color == ColorValue::INHERIT
                        && (local.contains_key(name)
                            || (nucleic && local.contains_key("cartoon_nucleic")))
                    {
                        color = ColorValue::BASE;
                    }
                } else if color == ColorValue::INHERIT && local.contains_key(name) {
                    // An explicit old -1 override masked a global representation color.
                    color = if fallback == ColorValue::INHERIT {
                        ColorValue::BASE
                    } else {
                        fallback
                    };
                }
            }
            *value = Value::from(color.to_bits());
        }
    }
    Ok(())
}

fn migrate_dynamic_settings(molecule: &mut Value, palette: &NamedPalette) -> Result<(), PrsError> {
    if let Some(Value::Map(values)) =
        field_mut(molecule, "settings", 7).and_then(|s| field_mut(s, "values", 0))
    {
        for (id, value) in values {
            migrate_setting_value(id.as_u64(), value, palette)?;
        }
    }
    if let Some(Value::Array(entries)) =
        field_mut(molecule, "unique_settings", 8).and_then(|s| field_mut(s, "entries", 1))
    {
        for entry in entries {
            let id = field(entry, "setting_id", 0).and_then(Value::as_u64);
            if let Some(value) = field_mut(entry, "value", 1) {
                migrate_setting_value(id, value, palette)?;
            }
        }
    }
    Ok(())
}

fn migrate_setting_value(
    id: Option<u64>,
    value: &mut Value,
    palette: &NamedPalette,
) -> Result<(), PrsError> {
    let is_color = id
        .and_then(|id| u16::try_from(id).ok())
        .and_then(patinae_settings::get_setting)
        .is_some_and(|s| s.setting_type == patinae_settings::SettingType::Color);
    if let Value::Map(variants) = value {
        if variants.len() == 1 {
            let (kind, payload) = &mut variants[0];
            if kind.as_str() == Some("Color") || (is_color && kind.as_str() == Some("Int")) {
                *payload = Value::from(setting_color(payload, palette)?.to_bits());
                *kind = Value::from("Color");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sentinels_and_enum_variants_have_context_specific_meanings() {
        let palette = NamedPalette::default();
        for (number, name, expected) in [
            (-1, "ByElement", ColorValue::ELEMENT),
            (-2, "ByChain", ColorValue::CHAIN),
            (-3, "BySS", ColorValue::SECONDARY_STRUCTURE),
            (-4, "ByBFactor", ColorValue::B_FACTOR),
            (-5, "ByResidueType", ColorValue::RESIDUE_TYPE),
            (-6, "ByResidueIndex", ColorValue::RESIDUE_INDEX),
        ] {
            assert_eq!(atom_color(&Value::from(number)).unwrap(), expected);
            assert_eq!(atom_color(&Value::from(name)).unwrap(), expected);
            assert_eq!(
                setting_color(&Value::from(number), &palette).unwrap(),
                ColorValue::INHERIT
            );
        }
        assert_eq!(
            atom_color(&Value::from(i32::MIN)).unwrap(),
            ColorValue::INHERIT
        );
        assert_eq!(
            atom_color(&Value::from("Atomic")).unwrap(),
            ColorValue::ELEMENT
        );
        assert_eq!(
            setting_color(&Value::from(2), &palette).unwrap(),
            ColorValue::palette(2)
        );
        assert_eq!(
            setting_color(&Value::from(0x123456), &palette).unwrap(),
            ColorValue::rgb(0x12, 0x34, 0x56)
        );
        assert!(atom_color(&Value::from(u64::MAX)).is_err());
        assert!(atom_color(&Value::from("UnknownScheme")).is_err());
    }

    #[test]
    fn omitted_atom_fields_keep_historical_defaults() {
        for mut colors in [Value::Map(vec![]), Value::Array(vec![])] {
            migrate_atom_colors(&mut colors, &Defaults::new(), &Defaults::new(), false).unwrap();
            let colors: patinae_mol::AtomColors = decode(&colors).unwrap();
            assert_eq!(colors.base, ColorValue::ELEMENT);
            assert_eq!(colors.cartoon, ColorValue::CHAIN);
            assert_eq!(colors.stick, ColorValue::INHERIT);
        }
    }

    #[test]
    fn explicit_nucleic_unset_masks_global_color_and_tracks_base() {
        let local = Defaults::from([("cartoon_nucleic".to_owned(), ColorValue::INHERIT)]);
        let global = Defaults::from([("cartoon_nucleic".to_owned(), ColorValue::palette(2))]);
        let mut colors = Value::Map(vec![(Value::from("cartoon"), Value::from(i32::MIN))]);
        migrate_atom_colors(&mut colors, &local, &global, true).unwrap();
        let mut atom = patinae_mol::Atom::new("P", patinae_mol::Element::Phosphorus);
        atom.repr.colors = decode(&colors).unwrap();
        atom.state.flags = patinae_mol::AtomFlags::NUCLEIC;
        assert_eq!(atom.repr.colors.cartoon, ColorValue::BASE);
        let named = NamedPalette::default();
        let palette = patinae_color::ThemedPalette::dark();
        let resolver = patinae_scene::ColorResolver::new(&named, &palette);
        let defaults = patinae_scene::RepColorDefaults {
            cartoon_nucleic: ColorValue::palette(2),
            ..Default::default()
        };
        assert_eq!(
            resolver.resolve_cartoon_with_defaults(&atom, &defaults),
            resolver.resolve_atom(&atom)
        );
        atom.repr.colors.base = ColorValue::rgb(1, 2, 3);
        assert_eq!(
            resolver.resolve_cartoon_with_defaults(&atom, &defaults),
            resolver.resolve_atom(&atom)
        );
    }
}
