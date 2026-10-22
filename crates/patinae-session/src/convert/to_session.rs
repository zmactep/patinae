//! Convert [`PseSession`] → [`Session`]

use std::sync::Arc;

use ahash::AHashMap;
use lin_alg::f32::{Mat4, Vec3};

use patinae_color::{Color, ColorValue, NamedPalette};
use patinae_mol::{
    Atom, AtomColors, AtomIndex, AtomRepresentation, AtomResidue, BondOrder, BondStereo, CoordSet,
    Element, ObjectMolecule, RepMask, ResidueKey, SecondaryStructure, Symmetry,
};
use patinae_scene::{MoleculeObject, Object, SceneView, Session};
use patinae_settings::{ObjectOverrides, SerializedSetting, SettingValue};

use super::pymol_colors::pymol_color_rgb;
use crate::pse::{
    PseAtom, PseColor, PseCoordSet, PseMolecule, PseNameEntry, PseObject, PseObjectData,
    PseSelection, PseSession, PseSettingValue, PseSymmetry,
};
use crate::SessionError;

// =============================================================================
// PSE → patinae color index remapping
// =============================================================================

/// Caches the mapping from PSE color indices to our NamedPalette indices.
struct PseColorMapper {
    cache: AHashMap<i64, ColorValue>,
    /// Session-defined custom colors (from the PSE "colors" list), keyed by PSE index.
    session_colors: AHashMap<i64, [f32; 3]>,
}

impl PseColorMapper {
    fn new(pse_colors: &[PseColor]) -> Self {
        let mut session_colors = AHashMap::new();
        for c in pse_colors {
            session_colors.insert(c.index, c.rgb);
        }
        Self {
            cache: AHashMap::new(),
            session_colors,
        }
    }

    /// Map a PSE color index to our color index.
    ///
    /// Negative PSE color controls inherit the application color default.
    /// Positive values are looked up first in session-defined colors, then in the
    /// built-in PSE color table, registered in `NamedPalette` if needed,
    /// and the resulting local index is returned.
    fn map(&mut self, pymol_index: i64, named_palette: &mut NamedPalette) -> ColorValue {
        if pymol_index < 0 {
            return ColorValue::INHERIT;
        }
        if let Some(&local) = self.cache.get(&pymol_index) {
            return local;
        }
        // Session colors take priority over built-in table
        let local = if let Some(&[r, g, b]) = self.session_colors.get(&pymol_index) {
            let color_name = format!("_pse_{}", pymol_index);
            ColorValue::palette(named_palette.register(&color_name, Color::new(r, g, b)))
        } else if let Some((r, g, b)) = pymol_color_rgb(pymol_index) {
            let color_name = format!("_pse_{}", pymol_index);
            ColorValue::palette(named_palette.register(&color_name, Color::new(r, g, b)))
        } else {
            // Unknown PSE index inherits the representation default.
            ColorValue::INHERIT
        };
        self.cache.insert(pymol_index, local);
        local
    }
}

/// Convert a parsed PSE session into a live [`Session`].
pub fn pse_to_session(pse: &PseSession) -> Result<Session, SessionError> {
    let mut session = Session::new();

    // --- Camera / View ---
    let view = convert_view(&pse.view);
    session.camera.set_view(view);

    // --- Named views ---
    for (name, view_arr) in &pse.view_dict {
        session.views.store_view(name, convert_view(view_arr));
    }

    // --- Custom colors ---
    for color in &pse.colors {
        session.named_palette.register(
            &color.name,
            Color::new(color.rgb[0], color.rgb[1], color.rgb[2]),
        );
    }

    // --- Color mapper ---
    let mut color_mapper = PseColorMapper::new(&pse.colors);

    // --- Global settings (after color_mapper, so color indices get remapped) ---
    session.settings = convert_settings(pse, &mut color_mapper, &mut session.named_palette);

    // --- Background color from legacy bg_rgb setting (ID 6) ---
    if let Some(color) = resolve_bg_color(pse, &mut color_mapper, &mut session.named_palette) {
        session.clear_color = color;
        session.clear_color_set = true;
    }

    // --- Objects and Selections ---
    let mut object_names: Vec<Option<String>> = Vec::new();

    for entry in &pse.names {
        match entry {
            None => object_names.push(None),
            Some(PseNameEntry::Object(obj)) => {
                object_names.push(Some(obj.name.clone()));
                convert_object(obj, pse, &mut session, &mut color_mapper)?;
            }
            Some(PseNameEntry::Selection(sel)) => {
                object_names.push(Some(sel.name.clone()));
            }
        }
    }

    // --- Selections (after all objects are registered) ---
    for entry in &pse.names {
        if let Some(PseNameEntry::Selection(sel)) = entry {
            convert_selection(sel, &object_names, &mut session);
        }
    }

    Ok(session)
}

// =============================================================================
// Settings
// =============================================================================

/// Convert a list of PSE settings to serialized settings (with color remapping).
fn serialize_pse_settings(
    pse_settings: &[crate::pse::PseSetting],
    color_mapper: &mut PseColorMapper,
    named_palette: &mut NamedPalette,
) -> Vec<SerializedSetting> {
    pse_settings
        .iter()
        .filter_map(|s| {
            let value = convert_setting_value(&s.value, color_mapper, named_palette)?;
            Some(SerializedSetting { id: s.id, value })
        })
        .collect()
}

fn convert_settings(
    pse: &PseSession,
    color_mapper: &mut PseColorMapper,
    named_palette: &mut NamedPalette,
) -> patinae_settings::groups::Settings {
    let serialized = serialize_pse_settings(&pse.settings, color_mapper, named_palette);
    patinae_settings::legacy::import_session(&serialized)
}

/// Extract the background color from PSE `bg_rgb` setting (ID 6).
fn resolve_bg_color(
    pse: &PseSession,
    color_mapper: &mut PseColorMapper,
    named_palette: &mut NamedPalette,
) -> Option<[f32; 3]> {
    let bg_setting = pse
        .settings
        .iter()
        .find(|s| s.id == patinae_settings::id::bg_rgb)?;
    if let PseSettingValue::Color(pse_index) = &bg_setting.value {
        let local_index = color_mapper.map(*pse_index, named_palette);
        if let Some(local_index) = local_index.palette_index() {
            return named_palette
                .get_by_index(local_index)
                .map(|c| c.to_array());
        }
    }
    None
}

fn convert_object_overrides(
    pse_settings: &[crate::pse::PseSetting],
    color_mapper: &mut PseColorMapper,
    named_palette: &mut NamedPalette,
) -> ObjectOverrides {
    let serialized = serialize_pse_settings(pse_settings, color_mapper, named_palette);
    patinae_settings::legacy::import_object_overrides(&serialized)
}

fn convert_setting_value(
    pse_val: &PseSettingValue,
    color_mapper: &mut PseColorMapper,
    named_palette: &mut NamedPalette,
) -> Option<SettingValue> {
    Some(match pse_val {
        PseSettingValue::Bool(b) => SettingValue::Bool(*b),
        PseSettingValue::Int(i) => SettingValue::Int(*i as i32),
        PseSettingValue::Float(f) => SettingValue::Float(*f as f32),
        PseSettingValue::Float3(a) => SettingValue::Float3([a[0] as f32, a[1] as f32, a[2] as f32]),
        PseSettingValue::Color(c) => SettingValue::Color(color_mapper.map(*c, named_palette)),
        PseSettingValue::String(s) => SettingValue::String(s.clone()),
    })
}

// =============================================================================
// View / Camera
// =============================================================================

/// Convert PSE 25-float view to [`SceneView`].
///
/// PSE layout (25-float view state):
/// - `[0..16]`  4×4 rotation matrix (column-major, matching GLM/Mat4 convention)
/// - `[16..19]` camera position
/// - `[19..22]` rotation origin
/// - `[22]`     front clip
/// - `[23]`     back clip
/// - `[24]`     FOV (negative = orthographic)
fn convert_view(view: &[f64; 25]) -> SceneView {
    // PSE stores the 4x4 rotation in column-major order — same as our Mat4.data
    let mut rotation = Mat4::new_identity();
    for (i, &v) in view.iter().enumerate().take(16) {
        rotation.data[i] = v as f32;
    }

    let fov_raw = view[24] as f32;

    SceneView {
        rotation,
        // PSE uses negative z for "camera in front of origin", patinae uses positive z.
        // Negate to convert between conventions.
        position: Vec3::new(-(view[16] as f32), -(view[17] as f32), -(view[18] as f32)),
        origin: Vec3::new(view[19] as f32, view[20] as f32, view[21] as f32),
        clip_front: view[22] as f32,
        clip_back: view[23] as f32,
        fov: if fov_raw < 0.0 {
            fov_raw.abs()
        } else if fov_raw > 0.0 {
            fov_raw
        } else {
            14.0
        },
    }
}

// =============================================================================
// Objects
// =============================================================================

fn convert_object(
    obj: &PseObject,
    pse: &PseSession,
    session: &mut Session,
    color_mapper: &mut PseColorMapper,
) -> Result<(), SessionError> {
    match &obj.data {
        PseObjectData::Molecule(mol) => {
            let mol_obj = convert_molecule(
                &obj.name,
                obj,
                mol,
                pse,
                color_mapper,
                &mut session.named_palette,
            )?;
            session.registry.add(mol_obj);
        }
        PseObjectData::Unsupported => {
            log::debug!(
                "Skipping unsupported object '{}' (type {})",
                obj.name,
                obj.type_code
            );
        }
    }
    Ok(())
}

fn convert_molecule(
    name: &str,
    obj: &PseObject,
    pse_mol: &PseMolecule,
    pse: &PseSession,
    color_mapper: &mut PseColorMapper,
    named_palette: &mut NamedPalette,
) -> Result<MoleculeObject, SessionError> {
    let mut mol = ObjectMolecule::with_capacity(name, pse_mol.atoms.len(), pse_mol.bonds.len());
    mol.discrete = pse_mol.discrete;

    // Symmetry
    if let Some(sym) = &pse_mol.symmetry {
        mol.symmetry = Some(convert_symmetry(sym));
    }

    // Atoms
    for pse_atom in &pse_mol.atoms {
        mol.add_atom(convert_atom(pse_atom, color_mapper, named_palette));
    }

    // Bonds
    for pse_bond in &pse_mol.bonds {
        let a1 = AtomIndex(pse_bond.index0 as u32);
        let a2 = AtomIndex(pse_bond.index1 as u32);
        let order = BondOrder::from_raw(pse_bond.order as i8);
        if let Ok(bi) = mol.add_bond(a1, a2, order) {
            if let Some(bond) = mol.get_bond_mut(bi) {
                bond.stereo = match pse_bond.stereo {
                    1 => BondStereo::Up,
                    2 => BondStereo::Down,
                    3 => BondStereo::Either,
                    _ => BondStereo::None,
                };
                if pse_bond.unique_id != 0 {
                    bond.unique_id = Some(pse_bond.unique_id as i32);
                    bond.has_setting = true;
                }
            }
        }
    }

    // Coordinate sets
    for cs in pse_mol.coord_sets.iter().flatten() {
        mol.add_coord_set(convert_coord_set(cs, pse_mol.atoms.len()));
    }

    // Move supported atom color settings into AtomColors after object defaults.
    // Bond-specific and non-color settings retain their original scope.
    let atom_ids: std::collections::HashSet<_> = pse_mol
        .atoms
        .iter()
        .map(|atom| atom.unique_id as i32)
        .filter(|&id| id != 0)
        .collect();
    let mut atom_color_settings: AHashMap<i32, Vec<(&str, ColorValue)>> = AHashMap::new();
    let unique_list: Vec<_> = pse
        .unique_settings
        .iter()
        .filter_map(|us| {
            let value = convert_setting_value(&us.value, color_mapper, named_palette)?;
            if atom_ids.contains(&(us.unique_id as i32)) {
                if let Some(definition) = patinae_settings::get_setting(us.setting_id) {
                    if AtomColors::representation_field(definition.name).is_some() {
                        if let Some(color) = value.as_color() {
                            atom_color_settings
                                .entry(us.unique_id as i32)
                                .or_default()
                                .push((definition.name, color));
                            return None;
                        }
                    }
                }
            }
            Some((us.unique_id as i32, us.setting_id, value))
        })
        .collect();
    mol.unique_settings.from_session_list(&unique_list);

    // Classify atoms
    mol.classify_atoms();

    // Compute object-level rep_mask as union of all per-atom reps
    let obj_rep_mask = pse_mol
        .atoms
        .iter()
        .fold(0u32, |acc, a| acc | a.visible_reps);

    // Use normal constructor (which sets up rendering state),
    // then override per-atom visible_reps from PSE data.
    let mut mol_obj = MoleculeObject::new(mol);
    // Restore per-atom reps and remapped colors from PSE
    for (i, pse_atom) in pse_mol.atoms.iter().enumerate() {
        if let Some(atom) = mol_obj.molecule_mut().get_atom_mut(AtomIndex(i as u32)) {
            atom.repr.visible_reps = RepMask(pse_atom.visible_reps);
            atom.repr.colors.base = color_mapper.map(pse_atom.color, named_palette);
        }
    }
    let state = mol_obj.state_mut();
    state.enabled = obj.visible;
    state.visible_reps = RepMask(obj_rep_mask);
    state.color = color_mapper.map(obj.color, named_palette);

    // Apply TTT matrix (molecule position/rotation from the session)
    if let Some(ref ttt) = obj.ttt {
        state.set_transform(ttt_to_mat4(ttt));
    }

    // Per-object settings overrides
    if !obj.settings.is_empty() {
        let overrides = convert_object_overrides(&obj.settings, color_mapper, named_palette);
        *mol_obj.get_or_create_overrides() = overrides;
        let mut settings = serialize_pse_settings(&obj.settings, color_mapper, named_palette);
        // A nucleic-specific default wins over the general cartoon default.
        settings.sort_by_key(|setting| {
            patinae_settings::get_setting(setting.id)
                .is_some_and(|definition| definition.name == "cartoon_nucleic_acid_color")
        });
        for setting in settings {
            let Some(definition) = patinae_settings::get_setting(setting.id) else {
                continue;
            };
            let Some(field) = AtomColors::representation_field(definition.name) else {
                continue;
            };
            let Some(color) = setting.value.as_color() else {
                continue;
            };
            if color == ColorValue::INHERIT {
                continue;
            }
            for atom in mol_obj.molecule_mut().atoms_mut() {
                if definition.name == "cartoon_nucleic_acid_color"
                    && !atom.state.flags.contains(patinae_mol::AtomFlags::NUCLEIC)
                {
                    continue;
                }
                *field(&mut atom.repr.colors) = color;
            }
        }
    }

    for settings in atom_color_settings.values_mut() {
        settings.sort_by_key(|(name, _)| *name == "cartoon_nucleic_acid_color");
    }
    for atom in mol_obj.molecule_mut().atoms_mut() {
        if let Some(settings) = atom
            .repr
            .unique_id
            .and_then(|id| atom_color_settings.get(&id))
        {
            for &(name, color) in settings {
                if color == ColorValue::INHERIT {
                    continue;
                }
                if name == "cartoon_nucleic_acid_color"
                    && !atom.state.flags.contains(patinae_mol::AtomFlags::NUCLEIC)
                {
                    continue;
                }
                if let Some(field) = AtomColors::representation_field(name) {
                    *field(&mut atom.repr.colors) = color;
                }
            }
        }
    }
    Ok(mol_obj)
}

fn convert_atom(
    pse: &PseAtom,
    color_mapper: &mut PseColorMapper,
    named_palette: &mut NamedPalette,
) -> Atom {
    let element = Element::from_symbol(&pse.elem).unwrap_or(Element::Unknown);
    let inscode = pse
        .resi
        .chars()
        .last()
        .filter(|c| c.is_ascii_alphabetic())
        .unwrap_or(' ');
    let residue = Arc::new(AtomResidue::new(
        ResidueKey::new(&pse.chain, &pse.resn, pse.resv as i32, inscode),
        pse.segi.clone(),
    ));
    let ss_type = match pse.ss.as_str() {
        "H" => SecondaryStructure::Helix,
        "S" => SecondaryStructure::Sheet,
        _ => SecondaryStructure::Loop,
    };

    let mut atom = Atom {
        name: Arc::from(pse.name.as_str()),
        element,
        residue,
        alt: pse.alt.chars().next().unwrap_or(' '),
        b_factor: pse.b as f32,
        occupancy: pse.q as f32,
        vdw: pse.vdw as f32,
        partial_charge: pse.partial_charge as f32,
        formal_charge: pse.formal_charge as i8,
        ss_type,
        id: pse.id as i32,
        repr: AtomRepresentation {
            colors: AtomColors::with_base(color_mapper.map(pse.color, named_palette)),
            visible_reps: RepMask(pse.visible_reps),
            text_type: pse.text_type.clone(),
            label: pse.label.clone(),
            unique_id: if pse.unique_id != 0 {
                Some(pse.unique_id as i32)
            } else {
                None
            },
            has_setting: pse.unique_id != 0,
            ..Default::default()
        },
        ..Default::default()
    };
    atom.state.hetatm = pse.hetatm;
    atom
}

fn convert_coord_set(pse: &PseCoordSet, n_atoms: usize) -> CoordSet {
    if pse.idx_to_atm.is_empty() || pse.idx_to_atm.len() == n_atoms {
        // Check if idx_to_atm is identity (or empty) — no reordering needed
        let is_identity =
            pse.idx_to_atm.is_empty() || pse.idx_to_atm.iter().enumerate().all(|(i, &v)| v == i);

        if is_identity {
            let coords: Vec<f32> = pse.coords.iter().map(|&c| c as f32).collect();
            return CoordSet::from_coords(coords);
        }
    }

    // Reorder coordinates: PSE stores coords in index order,
    // but we need them in atom order. idx_to_atm[idx] = atom_index.
    let mut atom_coords = vec![0.0f32; n_atoms * 3];
    for (idx, &atm) in pse.idx_to_atm.iter().enumerate() {
        if atm < n_atoms && idx * 3 + 2 < pse.coords.len() {
            atom_coords[atm * 3] = pse.coords[idx * 3] as f32;
            atom_coords[atm * 3 + 1] = pse.coords[idx * 3 + 1] as f32;
            atom_coords[atm * 3 + 2] = pse.coords[idx * 3 + 2] as f32;
        }
    }
    CoordSet::from_coords(atom_coords)
}

fn convert_symmetry(pse: &PseSymmetry) -> Symmetry {
    Symmetry::new(
        &pse.space_group,
        [pse.cell[0] as f32, pse.cell[1] as f32, pse.cell[2] as f32],
        [pse.cell[3] as f32, pse.cell[4] as f32, pse.cell[5] as f32],
    )
}

// =============================================================================
// Selections
// =============================================================================

fn convert_selection(sel: &PseSelection, object_names: &[Option<String>], session: &mut Session) {
    // Build a union expression from referenced object names.
    // This is approximate — PSE selections store per-atom membership,
    // but SelectionManager uses text expressions.
    let obj_refs: Vec<&str> = sel
        .members
        .iter()
        .filter_map(|(idx, _)| object_names.get(*idx)?.as_deref())
        .collect();

    let expression = if obj_refs.is_empty() {
        "none".to_string()
    } else {
        obj_refs.join(" or ")
    };

    session.selections.define(&sel.name, &expression);
    session.selections.set_visible(&sel.name, sel.visible);
}

// =============================================================================
// TTT helpers
// =============================================================================

/// Convert a PSE TTT (16-float) matrix to a column-major [`Mat4`].
///
/// TTT format: `y = R * (x + pre_trans) + post_trans`
///   - `[0..3, 4..7, 8..11]` = 3×3 rotation (row-major) with post-translation in column 3
///   - `[12..15]` = pre-translation (applied before rotation)
///
/// The equivalent standard affine transform is `y = R*x + (R*pre + post)`.
fn ttt_to_mat4(ttt: &[f64; 16]) -> Mat4 {
    // r is row-major 3×3: r[row*3 + col]
    let r = [
        ttt[0] as f32,
        ttt[1] as f32,
        ttt[2] as f32,
        ttt[4] as f32,
        ttt[5] as f32,
        ttt[6] as f32,
        ttt[8] as f32,
        ttt[9] as f32,
        ttt[10] as f32,
    ];
    let pre = [ttt[12] as f32, ttt[13] as f32, ttt[14] as f32];

    // Effective translation = R * pre_trans + post_trans
    let tx = r[0] * pre[0] + r[1] * pre[1] + r[2] * pre[2] + ttt[3] as f32;
    let ty = r[3] * pre[0] + r[4] * pre[1] + r[5] * pre[2] + ttt[7] as f32;
    let tz = r[6] * pre[0] + r[7] * pre[1] + r[8] * pre[2] + ttt[11] as f32;

    // Column-major Mat4: data[col*4 + row]
    // r[row*3+col] → M[row,col] → data[col*4 + row]
    Mat4::new([
        r[0], r[3], r[6], 0.0, // col 0: M[0,0], M[1,0], M[2,0]
        r[1], r[4], r[7], 0.0, // col 1: M[0,1], M[1,1], M[2,1]
        r[2], r[5], r[8], 0.0, // col 2: M[0,2], M[1,2], M[2,2]
        tx, ty, tz, 1.0, // col 3: translation
    ])
}

// =============================================================================
// Color helpers
// =============================================================================

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pse::*;
    use std::collections::HashMap;

    fn minimal_pse() -> PseSession {
        PseSession {
            version: 1830000,
            settings: vec![],
            view: [
                // 4x4 identity rotation (column-major)
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                // position
                0.0, 0.0, -50.0, // origin
                0.0, 0.0, 0.0, // front clip, back clip, fov
                40.0, 100.0, 14.0,
            ],
            names: vec![],
            colors: vec![],
            view_dict: HashMap::new(),
            scene_order: vec![],
            unique_settings: vec![],
        }
    }

    #[test]
    fn test_empty_session() {
        let session = pse_to_session(&minimal_pse()).unwrap();
        assert_eq!(session.registry.len(), 0);
    }

    #[test]
    fn test_view_conversion() {
        let session = pse_to_session(&minimal_pse()).unwrap();
        let view = session.camera.current_view();
        assert!((view.position.z - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_custom_colors() {
        let mut pse = minimal_pse();
        pse.colors.push(PseColor {
            name: "mycolor".into(),
            index: 7777,
            rgb: [0.5, 0.3, 0.1],
        });
        let session = pse_to_session(&pse).unwrap();
        let (_, c) = session.named_palette.get_by_name("mycolor").unwrap();
        assert!((c.r - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_molecule_conversion() {
        let mut pse = minimal_pse();
        pse.names
            .push(Some(PseNameEntry::Object(Box::new(PseObject {
                name: "mol1".into(),
                type_code: 1,
                visible: true,
                rep_mask: 0x01,
                color: -1,
                ttt: None,
                settings: vec![],
                data: PseObjectData::Molecule(PseMolecule {
                    atoms: vec![PseAtom {
                        resv: 1,
                        chain: "A".into(),
                        alt: String::new(),
                        resi: "1".into(),
                        segi: String::new(),
                        resn: "ALA".into(),
                        name: "CA".into(),
                        elem: "C".into(),
                        text_type: String::new(),
                        label: String::new(),
                        ss: "H".into(),
                        b: 20.0,
                        q: 1.0,
                        vdw: 1.7,
                        partial_charge: 0.0,
                        formal_charge: 0,
                        color: -1,
                        id: 1,
                        unique_id: 0,
                        visible_reps: 0x20,
                        hetatm: false,
                    }],
                    bonds: vec![],
                    coord_sets: vec![Some(PseCoordSet {
                        n_atom: 1,
                        coords: vec![1.0, 2.0, 3.0],
                        idx_to_atm: vec![],
                    })],
                    discrete: false,
                    n_discrete: 0,
                    symmetry: None,
                }),
            }))));

        let Some(PseNameEntry::Object(object)) = pse.names[0].as_mut() else {
            unreachable!()
        };
        object.settings = vec![
            PseSetting {
                id: 236,
                type_code: 5,
                value: PseSettingValue::Color(2),
            },
            PseSetting {
                id: 376,
                type_code: 5,
                value: PseSettingValue::Color(2),
            },
            PseSetting {
                id: 451,
                type_code: 5,
                value: PseSettingValue::Color(3),
            },
        ];
        let PseObjectData::Molecule(molecule) = &mut object.data else {
            unreachable!()
        };
        molecule.atoms[0].unique_id = 42;
        pse.unique_settings.push(PseUniqueSetting {
            unique_id: 42,
            setting_id: 376,
            type_code: 5,
            value: PseSettingValue::Color(4),
        });
        pse.unique_settings.push(PseUniqueSetting {
            unique_id: 42,
            setting_id: 236,
            type_code: 5,
            value: PseSettingValue::Color(-1),
        });
        let session = pse_to_session(&pse).unwrap();
        assert_eq!(session.registry.len(), 1);
        let mol_obj = session.registry.get_molecule("mol1").unwrap();
        assert_eq!(mol_obj.molecule().atom_count(), 1);
        let colors = &mol_obj.molecule().atoms_slice()[0].repr.colors;
        assert_eq!(
            colors.stick,
            session.named_palette.parse_color("_pse_4").unwrap()
        );
        // The nucleic object setting must not affect this protein atom.
        assert_eq!(
            colors.cartoon,
            session.named_palette.parse_color("_pse_2").unwrap()
        );
        assert!(mol_obj.molecule().unique_settings.get(42, 376).is_none());
        assert_eq!(
            &*mol_obj.molecule().get_atom(AtomIndex(0)).unwrap().name,
            "CA"
        );
    }

    #[test]
    fn test_selection_conversion() {
        let mut pse = minimal_pse();
        pse.names
            .push(Some(PseNameEntry::Object(Box::new(PseObject {
                name: "mol1".into(),
                type_code: 1,
                visible: true,
                rep_mask: 1,
                color: -1,
                ttt: None,
                settings: vec![],
                data: PseObjectData::Molecule(PseMolecule {
                    atoms: vec![],
                    bonds: vec![],
                    coord_sets: vec![],
                    discrete: false,
                    n_discrete: 0,
                    symmetry: None,
                }),
            }))));
        pse.names.push(Some(PseNameEntry::Selection(PseSelection {
            name: "sele".into(),
            visible: true,
            members: vec![(0, vec![0])],
        })));

        let session = pse_to_session(&pse).unwrap();
        assert!(session.selections.contains("sele"));
    }

    #[test]
    fn test_named_views() {
        let mut pse = minimal_pse();
        pse.view_dict.insert(
            "front".into(),
            [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                0.0, 0.0, -30.0, 0.0, 0.0, 0.0, 20.0, 80.0, 14.0,
            ],
        );
        let session = pse_to_session(&pse).unwrap();
        assert!(session.views.get("front").is_some());
    }

    #[test]
    fn test_ttt_matrix_applied() {
        let mut pse = minimal_pse();
        pse.names
            .push(Some(PseNameEntry::Object(Box::new(PseObject {
                name: "mol1".into(),
                type_code: 1,
                visible: true,
                rep_mask: 0x01,
                color: -1,
                ttt: Some([
                    1.0, 0.0, 0.0, 5.0, // identity rotation + 5.0 X post-translation
                    0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0,
                    1.0, // no pre-translation
                ]),
                settings: vec![],
                data: PseObjectData::Molecule(PseMolecule {
                    atoms: vec![PseAtom {
                        resv: 1,
                        chain: "A".into(),
                        alt: String::new(),
                        resi: "1".into(),
                        segi: String::new(),
                        resn: "ALA".into(),
                        name: "CA".into(),
                        elem: "C".into(),
                        text_type: String::new(),
                        label: String::new(),
                        ss: String::new(),
                        b: 0.0,
                        q: 1.0,
                        vdw: 1.7,
                        partial_charge: 0.0,
                        formal_charge: 0,
                        color: -1,
                        id: 1,
                        unique_id: 0,
                        visible_reps: 0x20,
                        hetatm: false,
                    }],
                    bonds: vec![],
                    coord_sets: vec![Some(PseCoordSet {
                        n_atom: 1,
                        coords: vec![0.0, 0.0, 0.0],
                        idx_to_atm: vec![],
                    })],
                    discrete: false,
                    n_discrete: 0,
                    symmetry: None,
                }),
            }))));

        let session = pse_to_session(&pse).unwrap();
        let mol_obj = session.registry.get_molecule("mol1").unwrap();
        // Transform should have the 5.0 X translation (column-major: data[12])
        let t = &mol_obj.state().transform;
        assert!(
            (t.data[12] - 5.0).abs() < 1e-6,
            "expected tx=5.0, got {}",
            t.data[12]
        );
    }

    #[test]
    fn test_no_ttt_means_identity() {
        let mut pse = minimal_pse();
        pse.names
            .push(Some(PseNameEntry::Object(Box::new(PseObject {
                name: "mol1".into(),
                type_code: 1,
                visible: true,
                rep_mask: 1,
                color: -1,
                ttt: None,
                settings: vec![],
                data: PseObjectData::Molecule(PseMolecule {
                    atoms: vec![],
                    bonds: vec![],
                    coord_sets: vec![],
                    discrete: false,
                    n_discrete: 0,
                    symmetry: None,
                }),
            }))));

        let session = pse_to_session(&pse).unwrap();
        let mol_obj = session.registry.get_molecule("mol1").unwrap();
        assert_eq!(mol_obj.state().transform.data, Mat4::new_identity().data);
    }

    #[test]
    fn test_session_custom_color_mapping() {
        let mut pse = minimal_pse();
        // Register a custom session color at PSE index 9999
        pse.colors.push(PseColor {
            name: "custom_blue".into(),
            index: 9999,
            rgb: [0.0, 0.0, 1.0],
        });
        // Create a molecule with an atom using that custom color index
        pse.names
            .push(Some(PseNameEntry::Object(Box::new(PseObject {
                name: "mol1".into(),
                type_code: 1,
                visible: true,
                rep_mask: 1,
                color: -1,
                ttt: None,
                settings: vec![],
                data: PseObjectData::Molecule(PseMolecule {
                    atoms: vec![PseAtom {
                        resv: 1,
                        chain: "A".into(),
                        alt: String::new(),
                        resi: "1".into(),
                        segi: String::new(),
                        resn: "ALA".into(),
                        name: "CA".into(),
                        elem: "C".into(),
                        text_type: String::new(),
                        label: String::new(),
                        ss: String::new(),
                        b: 0.0,
                        q: 1.0,
                        vdw: 1.7,
                        partial_charge: 0.0,
                        formal_charge: 0,
                        color: 9999,
                        id: 1,
                        unique_id: 0,
                        visible_reps: 0x20,
                        hetatm: false,
                    }],
                    bonds: vec![],
                    coord_sets: vec![Some(PseCoordSet {
                        n_atom: 1,
                        coords: vec![0.0, 0.0, 0.0],
                        idx_to_atm: vec![],
                    })],
                    discrete: false,
                    n_discrete: 0,
                    symmetry: None,
                }),
            }))));

        let session = pse_to_session(&pse).unwrap();
        let mol_obj = session.registry.get_molecule("mol1").unwrap();
        let atom = mol_obj.molecule().get_atom(AtomIndex(0)).unwrap();
        // The atom's base color should reference the imported palette entry.
        assert!(
            atom.repr.colors.base.palette_index().is_some(),
            "expected mapped color index, got {}",
            atom.repr.colors.base
        );
    }

    #[test]
    fn test_ttt_with_pre_translation() {
        // TTT with pre-translation: y = R * (x + pre) + post
        // With identity rotation: y = x + pre + post
        let ttt: [f64; 16] = [
            1.0, 0.0, 0.0, 3.0, // post_x = 3
            0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 2.0, 0.0, 0.0, 1.0, // pre_x = 2
        ];
        let mat = ttt_to_mat4(&ttt);
        // Effective tx = R*pre + post = 1.0*2.0 + 3.0 = 5.0
        // Column-major: translation at data[12]
        assert!((mat.data[12] - 5.0).abs() < 1e-6);
    }
}
