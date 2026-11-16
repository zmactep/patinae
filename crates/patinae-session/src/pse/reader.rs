use std::collections::HashMap;

use crate::pickle::PickleValue;
use crate::SessionError;

use super::*;

/// Convert a top-level [`PickleValue`] (expected to be a Dict) into a [`PseSession`].
pub fn read_session(value: &PickleValue) -> Result<PseSession, SessionError> {
    let _dict = value
        .as_dict()
        .ok_or_else(|| SessionError::Pse("top-level value is not a dict".into()))?;

    let version = value.get("version").and_then(|v| v.as_int()).unwrap_or(0);

    let settings = read_settings(value.get("settings"))?;
    let view = read_view(value.get("view"))?;
    let names = read_names(value.get("names"))?;
    let colors = read_colors(value.get("colors"))?;
    let view_dict = read_view_dict(value.get("view_dict"))?;
    let scene_order = read_scene_order(value.get("scene_order"))?;
    let unique_settings = read_unique_settings(value.get("unique_settings"))?;

    Ok(PseSession {
        version,
        settings,
        view,
        names,
        colors,
        view_dict,
        scene_order,
        unique_settings,
    })
}

fn read_settings(value: Option<&PickleValue>) -> Result<Vec<PseSetting>, SessionError> {
    let list = match value {
        Some(v) => match v.as_list().or_else(|| v.as_tuple()) {
            Some(l) => l,
            None => return Ok(Vec::new()),
        },
        None => return Ok(Vec::new()),
    };

    let mut settings = Vec::new();
    for item in list {
        let tup = item
            .as_list()
            .or_else(|| item.as_tuple())
            .ok_or_else(|| SessionError::Pse("setting entry is not a list/tuple".into()))?;
        if tup.len() < 3 {
            continue;
        }
        let id = tup[0].as_int().unwrap_or(0) as u16;
        let type_code = tup[1].as_int().unwrap_or(0) as u8;
        let value = read_setting_value(type_code, &tup[2])?;
        settings.push(PseSetting {
            id,
            type_code,
            value,
        });
    }
    Ok(settings)
}

fn read_setting_value(type_code: u8, val: &PickleValue) -> Result<PseSettingValue, SessionError> {
    Ok(match type_code {
        1 => PseSettingValue::Bool(val.as_bool().unwrap_or(false)),
        2 => PseSettingValue::Int(val.as_int().unwrap_or(0)),
        3 => PseSettingValue::Float(val.as_float().unwrap_or(0.0)),
        4 => {
            let list = val.as_list().or_else(|| val.as_tuple()).unwrap_or(&[]);
            let mut arr = [0.0; 3];
            for (i, item) in list.iter().enumerate().take(3) {
                arr[i] = item.as_float().unwrap_or(0.0);
            }
            PseSettingValue::Float3(arr)
        }
        5 => PseSettingValue::Color(val.as_int().unwrap_or(0)),
        6 => PseSettingValue::String(val.as_str().unwrap_or("").to_string()),
        _ => PseSettingValue::Int(val.as_int().unwrap_or(0)),
    })
}

fn read_view(value: Option<&PickleValue>) -> Result<[f64; 25], SessionError> {
    let mut view = [0.0f64; 25];
    if let Some(v) = value {
        let list = v.as_list().or_else(|| v.as_tuple()).unwrap_or(&[]);
        for (i, item) in list.iter().enumerate().take(25) {
            view[i] = item.as_float().unwrap_or(0.0);
        }
    }
    Ok(view)
}

fn read_names(value: Option<&PickleValue>) -> Result<Vec<Option<PseNameEntry>>, SessionError> {
    let list = match value {
        Some(v) => v.as_list().unwrap_or(&[]),
        None => return Ok(Vec::new()),
    };

    let mut names = Vec::new();
    for item in list {
        if matches!(item, PickleValue::None) {
            names.push(None);
            continue;
        }
        let entry = item
            .as_list()
            .ok_or_else(|| SessionError::Pse("names entry is not a list".into()))?;
        if entry.len() < 7 {
            names.push(None);
            continue;
        }
        // PSE names entry layout:
        // [0] = name (str)
        // [1] = object state (int) — not the type code
        // [2] = selection members / 0
        // [3] = None / group info
        // [4] = type code (1=molecule, 2=selection, etc.)
        // [5] = object data (list) / rep_mask
        // [6] = color / group name
        let name = entry[0].as_str().unwrap_or("").to_string();
        let type_code = entry[4].as_int().unwrap_or(0);

        match type_code {
            2 => {
                // Selection
                let visible = entry.get(1).and_then(|v| v.as_int()).unwrap_or(0) != 0;
                let members = read_selection_members(&entry[5])?;
                names.push(Some(PseNameEntry::Selection(PseSelection {
                    name,
                    visible,
                    members,
                })));
            }
            _ => {
                // Object (type 1 = molecule, etc.)
                let (data, hdr) = if let Some(obj_data) = entry.get(5) {
                    read_object_data(type_code, obj_data)?
                } else {
                    (
                        PseObjectData::Unsupported,
                        PseObjectHeader {
                            visible: true,
                            color: 0,
                            rep_mask: 0,
                            ttt: None,
                            settings: vec![],
                        },
                    )
                };

                names.push(Some(PseNameEntry::Object(Box::new(PseObject {
                    name,
                    type_code,
                    visible: hdr.visible,
                    rep_mask: hdr.rep_mask,
                    color: hdr.color,
                    ttt: hdr.ttt,
                    settings: hdr.settings,
                    data,
                }))));
            }
        }
    }
    Ok(names)
}

/// Parsed fields from the PSE object header (CObject base).
struct PseObjectHeader {
    visible: bool,
    color: i64,
    rep_mask: u32,
    ttt: Option<[f64; 16]>,
    settings: Vec<PseSetting>,
}

/// Returns (data, header) extracted from the object data list.
///
/// The header at list[0] is a CObject with PSE format layout:
///   [0]=type, [1]=name, [2]=Color, [3]=visRep, [7]=TTTFlag, [9]=Enabled, [11]=TTT(16 floats)
fn read_object_data(
    type_code: i64,
    value: &PickleValue,
) -> Result<(PseObjectData, PseObjectHeader), SessionError> {
    let default_header = PseObjectHeader {
        visible: true,
        color: 0,
        rep_mask: 0,
        ttt: None,
        settings: vec![],
    };

    // Only parse molecules (type_code=1) for now
    if type_code != 1 {
        return Ok((PseObjectData::Unsupported, default_header));
    }

    let list = match value.as_list() {
        Some(l) if l.len() >= 10 => l,
        _ => return Ok((PseObjectData::Unsupported, default_header)),
    };

    // Extract fields from header (list[0]) per PSE format object layout
    let header = list[0].as_list();
    let visible = header
        .and_then(|h| h.get(9))
        .and_then(|v| v.as_int())
        .unwrap_or(1)
        != 0;
    let color = header
        .and_then(|h| h.get(2))
        .and_then(|v| v.as_int())
        .unwrap_or(0);
    let rep_mask = header
        .and_then(|h| h.get(3))
        .and_then(|v| v.as_int())
        .unwrap_or(0) as u32;
    let ttt_flag = header
        .and_then(|h| h.get(7))
        .and_then(|v| v.as_int())
        .unwrap_or(0)
        != 0;
    let ttt = if ttt_flag {
        header
            .and_then(|h| h.get(11))
            .and_then(|v| v.as_list().or_else(|| v.as_tuple()))
            .and_then(|ttt_list| {
                if ttt_list.len() >= 16 {
                    let mut arr = [0.0f64; 16];
                    for (i, item) in ttt_list.iter().enumerate().take(16) {
                        arr[i] = item.as_float().unwrap_or(0.0);
                    }
                    Some(arr)
                } else {
                    None
                }
            })
    } else {
        None
    };

    // Per-object settings from header[8] (PSE format object settings)
    let obj_settings = header
        .and_then(|h| h.get(8))
        .map(|v| read_settings(Some(v)).unwrap_or_default())
        .unwrap_or_default();

    // PSE molecule object data layout:
    // [0] = header/settings, [1] = 1, [2] = n_bonds, [3] = n_atoms,
    // [4] = coord_sets, [5] = symmetry, [6] = bonds, [7] = atoms,
    // [8] = discrete, [9] = n_discrete, [10]-[15] = misc
    let atoms = read_atoms(list.get(7))?;
    let bonds = read_bonds(list.get(6))?;
    let coord_sets = read_coord_sets(list.get(4))?;
    let discrete = list.get(8).and_then(|v| v.as_int()).unwrap_or(0) != 0;
    let n_discrete = list.get(9).and_then(|v| v.as_int()).unwrap_or(0);
    let symmetry = read_symmetry(list.get(5))?;

    Ok((
        PseObjectData::Molecule(PseMolecule {
            atoms,
            bonds,
            coord_sets,
            discrete,
            n_discrete,
            symmetry,
        }),
        PseObjectHeader {
            visible,
            color,
            rep_mask,
            ttt,
            settings: obj_settings,
        },
    ))
}

fn read_atoms(value: Option<&PickleValue>) -> Result<Vec<PseAtom>, SessionError> {
    let list = match value {
        Some(v) => v.as_list().unwrap_or(&[]),
        None => return Ok(Vec::new()),
    };

    let mut atoms = Vec::with_capacity(list.len());
    for item in list {
        let a = item.as_list().or_else(|| item.as_tuple()).unwrap_or(&[]);
        let get_str = |idx: usize| -> String {
            a.get(idx)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let get_int = |idx: usize| -> i64 { a.get(idx).and_then(|v| v.as_int()).unwrap_or(0) };
        let get_float =
            |idx: usize| -> f64 { a.get(idx).and_then(|v| v.as_float()).unwrap_or(0.0) };

        atoms.push(PseAtom {
            resv: get_int(0),
            chain: get_str(1),
            alt: get_str(2),
            resi: get_str(3),
            segi: get_str(4),
            resn: get_str(5),
            name: get_str(6),
            elem: get_str(7),
            text_type: get_str(8),
            label: get_str(9),
            ss: get_str(10),
            id: get_int(22),
            b: get_float(14),
            q: get_float(15),
            vdw: get_float(16),
            partial_charge: get_float(17),
            formal_charge: get_int(11),
            color: get_int(21),
            unique_id: get_int(37).max(0) as u64,
            visible_reps: get_int(20) as u32,
            hetatm: get_int(23) != 0,
        });
    }
    Ok(atoms)
}

fn read_bonds(value: Option<&PickleValue>) -> Result<Vec<PseBond>, SessionError> {
    let list = match value {
        Some(v) => v.as_list().unwrap_or(&[]),
        None => return Ok(Vec::new()),
    };

    let mut bonds = Vec::with_capacity(list.len());
    for item in list {
        let b = item.as_list().or_else(|| item.as_tuple()).unwrap_or(&[]);
        let get_int = |idx: usize| -> i64 { b.get(idx).and_then(|v| v.as_int()).unwrap_or(0) };

        bonds.push(PseBond {
            index0: get_int(0).max(0) as usize,
            index1: get_int(1).max(0) as usize,
            order: get_int(2),
            id: get_int(3),
            stereo: get_int(4),
            unique_id: get_int(5).max(0) as u64,
        });
    }
    Ok(bonds)
}

fn read_coord_sets(value: Option<&PickleValue>) -> Result<Vec<Option<PseCoordSet>>, SessionError> {
    let list = match value {
        Some(v) => v.as_list().unwrap_or(&[]),
        None => return Ok(Vec::new()),
    };

    let mut sets = Vec::new();
    for item in list {
        if matches!(item, PickleValue::None) {
            sets.push(None);
            continue;
        }
        let cs = item.as_list().or_else(|| item.as_tuple()).unwrap_or(&[]);
        if cs.is_empty() {
            sets.push(None);
            continue;
        }
        let n_atom = cs[0].as_int().unwrap_or(0).max(0) as usize;
        let coords = cs
            .get(2)
            .and_then(|v| v.as_list())
            .unwrap_or(&[])
            .iter()
            .map(|v| v.as_float().unwrap_or(0.0))
            .collect();
        let idx_to_atm = cs
            .get(3)
            .and_then(|v| v.as_list())
            .unwrap_or(&[])
            .iter()
            .map(|v| v.as_int().unwrap_or(0).max(0) as usize)
            .collect();
        sets.push(Some(PseCoordSet {
            n_atom,
            coords,
            idx_to_atm,
        }));
    }
    Ok(sets)
}

fn read_symmetry(value: Option<&PickleValue>) -> Result<Option<PseSymmetry>, SessionError> {
    let v = match value {
        Some(v) if !matches!(v, PickleValue::None) => v,
        _ => return Ok(None),
    };
    let list = v.as_list().or_else(|| v.as_tuple()).unwrap_or(&[]);
    if list.is_empty() {
        return Ok(None);
    }

    // Cell parameters are in the first element (list of 6 floats)
    let cell_list = list[0]
        .as_list()
        .or_else(|| list[0].as_tuple())
        .unwrap_or(&[]);
    let mut cell = [0.0f64; 6];
    for (i, item) in cell_list.iter().enumerate().take(6) {
        cell[i] = item.as_float().unwrap_or(0.0);
    }

    let space_group = list
        .get(1)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok(Some(PseSymmetry { cell, space_group }))
}

fn read_selection_members(value: &PickleValue) -> Result<Vec<(usize, Vec<usize>)>, SessionError> {
    let list = value.as_list().unwrap_or(&[]);
    let mut members = Vec::new();
    for item in list {
        let pair = item.as_list().or_else(|| item.as_tuple()).unwrap_or(&[]);
        if pair.len() < 2 {
            continue;
        }
        let obj_idx = pair[0].as_int().unwrap_or(0).max(0) as usize;
        let atom_indices: Vec<usize> = pair[1]
            .as_list()
            .unwrap_or(&[])
            .iter()
            .filter_map(|v| v.as_int().map(|i| i.max(0) as usize))
            .collect();
        members.push((obj_idx, atom_indices));
    }
    Ok(members)
}

fn read_colors(value: Option<&PickleValue>) -> Result<Vec<PseColor>, SessionError> {
    let list = match value {
        Some(v) => v.as_list().unwrap_or(&[]),
        None => return Ok(Vec::new()),
    };
    let mut colors = Vec::new();
    for item in list {
        let entry = item.as_list().or_else(|| item.as_tuple()).unwrap_or(&[]);
        // PSE color list format: [name, index, [r,g,b], custom, lut_flag, [lut_rgb], fixed]
        if entry.len() < 3 {
            continue;
        }
        let name = entry[0].as_str().unwrap_or("").to_string();
        let index = entry[1].as_int().unwrap_or(0);
        let rgb_list = entry[2]
            .as_list()
            .or_else(|| entry[2].as_tuple())
            .unwrap_or(&[]);
        let mut rgb = [0.0f32; 3];
        for (i, item) in rgb_list.iter().enumerate().take(3) {
            rgb[i] = item.as_float().unwrap_or(0.0) as f32;
        }
        colors.push(PseColor { name, index, rgb });
    }
    Ok(colors)
}

fn read_view_dict(value: Option<&PickleValue>) -> Result<HashMap<String, [f64; 25]>, SessionError> {
    let dict = match value {
        Some(v) => v.as_dict().unwrap_or(&[]),
        None => return Ok(HashMap::new()),
    };
    let mut map = HashMap::new();
    for (k, v) in dict {
        let name = k.as_str().unwrap_or("").to_string();
        let list = v.as_list().or_else(|| v.as_tuple()).unwrap_or(&[]);
        let mut view = [0.0f64; 25];
        for (i, item) in list.iter().enumerate().take(25) {
            view[i] = item.as_float().unwrap_or(0.0);
        }
        map.insert(name, view);
    }
    Ok(map)
}

fn read_scene_order(value: Option<&PickleValue>) -> Result<Vec<String>, SessionError> {
    let list = match value {
        Some(v) => v.as_list().unwrap_or(&[]),
        None => return Ok(Vec::new()),
    };
    let mut scenes = Vec::new();
    for item in list {
        // scene_order entries are (name, display_string) tuples
        if let Some(tup) = item.as_list().or_else(|| item.as_tuple()) {
            if let Some(name) = tup.first().and_then(|v| v.as_str()) {
                scenes.push(name.to_string());
            }
        } else if let Some(name) = item.as_str() {
            scenes.push(name.to_string());
        }
    }
    Ok(scenes)
}

fn read_unique_settings(
    value: Option<&PickleValue>,
) -> Result<Vec<PseUniqueSetting>, SessionError> {
    let list = match value {
        Some(v) => v.as_list().unwrap_or(&[]),
        None => return Ok(Vec::new()),
    };
    let mut settings = Vec::new();
    for item in list {
        let tup = item.as_list().or_else(|| item.as_tuple()).unwrap_or(&[]);
        if tup.len() < 4 {
            continue;
        }
        let unique_id = tup[0].as_int().unwrap_or(0).max(0) as u64;
        let setting_id = tup[1].as_int().unwrap_or(0) as u16;
        let type_code = tup[2].as_int().unwrap_or(0) as u8;
        let value = read_setting_value(type_code, &tup[3])?;
        settings.push(PseUniqueSetting {
            unique_id,
            setting_id,
            type_code,
            value,
        });
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pickle::PickleValue;

    #[test]
    fn test_read_minimal_session() {
        let session = PickleValue::Dict(vec![
            (
                PickleValue::String("version".into()),
                PickleValue::Int(1830000),
            ),
            (
                PickleValue::String("settings".into()),
                PickleValue::List(vec![]),
            ),
            (
                PickleValue::String("view".into()),
                PickleValue::Tuple((0..25).map(|i| PickleValue::Float(i as f64)).collect()),
            ),
            (
                PickleValue::String("names".into()),
                PickleValue::List(vec![]),
            ),
        ]);

        let pse = read_session(&session).unwrap();
        assert_eq!(pse.version, 1830000);
        assert!(pse.settings.is_empty());
        assert!(pse.names.is_empty());
        for (i, value) in pse.view.iter().enumerate() {
            assert!((value - i as f64).abs() < 1e-10);
        }
    }

    #[test]
    fn test_read_setting() {
        let session = PickleValue::Dict(vec![(
            PickleValue::String("settings".into()),
            PickleValue::List(vec![PickleValue::List(vec![
                PickleValue::Int(10),
                PickleValue::Int(3),
                PickleValue::Float(1.5),
            ])]),
        )]);

        let pse = read_session(&session).unwrap();
        assert_eq!(pse.settings.len(), 1);
        assert_eq!(pse.settings[0].id, 10);
        assert_eq!(pse.settings[0].type_code, 3);
        assert_eq!(pse.settings[0].value, PseSettingValue::Float(1.5));
    }

    #[test]
    fn test_read_colors() {
        let session = PickleValue::Dict(vec![(
            PickleValue::String("colors".into()),
            PickleValue::List(vec![PickleValue::List(vec![
                PickleValue::String("red".into()),
                PickleValue::Int(1),
                PickleValue::List(vec![
                    PickleValue::Float(1.0),
                    PickleValue::Float(0.0),
                    PickleValue::Float(0.0),
                ]),
            ])]),
        )]);

        let pse = read_session(&session).unwrap();
        assert_eq!(pse.colors.len(), 1);
        assert_eq!(pse.colors[0].name, "red");
        assert_eq!(pse.colors[0].index, 1);
        assert!((pse.colors[0].rgb[0] - 1.0).abs() < 1e-6);
    }
}
