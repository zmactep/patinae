//! CPU analytic and semantic-sample export for display geometry.
//!
//! GPU-generated mesh readback is owned by `render_state::geometry_export_flow`;
//! this module only exports geometry that can be reconstructed from host-side
//! molecule/settings data without touching renderer buffers.

use std::collections::HashMap;

use patinae_mol::{AtomIndex, BondOrder, CoordSet, ObjectMolecule, RepMask};
use patinae_settings::ResolvedSettings;

#[cfg(test)]
use crate::picking::ObjectId;
use crate::picking::RepKind;
use crate::render_input::{RenderObjectInput, RepColorLutEntry, REP_COLOR_INHERIT};

use super::{
    DisplayedMaterial, DisplayedObjectGeometry, DisplayedPrimitive, GeometryExportOptions,
};

const DOT_INSTANCE_SIZE: u64 = 32;
const MAX_DOT_BUFFER_BYTES: u64 = 2_000_000_000;
const MAX_DOT_INSTANCES: u64 = MAX_DOT_BUFFER_BYTES / DOT_INSTANCE_SIZE;
const GOLDEN_ANGLE: f32 = 2.399_963_1;
const VALENCE_JOIN_FACTOR: f32 = 0.35;

pub(crate) fn append_cpu_object_geometry(
    object: &mut DisplayedObjectGeometry,
    input: &RenderObjectInput<'_>,
    scene_settings: &ResolvedSettings,
    options: &GeometryExportOptions,
) {
    let settings = input.object_settings.as_ref().unwrap_or(scene_settings);
    let first_new_primitive = object.primitives.len();
    if options.include_analytic {
        append_spheres(object, input, settings);
        append_sticks(object, input, settings);
    }
    if options.include_semantic_samples {
        append_lines(object, input, settings);
        append_dots(object, input, settings);
    }
    for primitive in &mut object.primitives[first_new_primitive..] {
        transform_primitive(primitive, &input.transform);
    }
}

fn transform_primitive(primitive: &mut DisplayedPrimitive, transform: &[[f32; 4]; 4]) {
    match primitive {
        DisplayedPrimitive::Mesh { mesh, .. } => {
            for vertex in &mut mesh.vertices {
                vertex.position = transform_point(transform, vertex.position);
                vertex.normal = transform_dir(transform, vertex.normal);
            }
        }
        DisplayedPrimitive::Sphere { center, .. } => {
            *center = transform_point(transform, *center);
        }
        DisplayedPrimitive::Cylinder { start, end, .. }
        | DisplayedPrimitive::LineSegment { start, end, .. } => {
            *start = transform_point(transform, *start);
            *end = transform_point(transform, *end);
        }
        DisplayedPrimitive::PointSample { position, .. } => {
            *position = transform_point(transform, *position);
        }
    }
}

fn transform_point(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
    ]
}

fn transform_dir(m: &[[f32; 4]; 4], n: [f32; 3]) -> [f32; 3] {
    normalize3([
        m[0][0] * n[0] + m[1][0] * n[1] + m[2][0] * n[2],
        m[0][1] * n[0] + m[1][1] * n[1] + m[2][1] * n[2],
        m[0][2] * n[0] + m[1][2] * n[1] + m[2][2] * n[2],
    ])
    .unwrap_or([0.0, 0.0, 1.0])
}

pub(crate) fn material_for_atom(
    input: &RenderObjectInput<'_>,
    scene_settings: &ResolvedSettings,
    rep: RepKind,
    atom_id: u32,
) -> DisplayedMaterial {
    let base = input
        .atom_colors
        .get(atom_id as usize)
        .copied()
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let rep_override = input
        .atom_rep_colors
        .get(atom_id as usize)
        .copied()
        .unwrap_or_default();
    let packed = rep_packed_color(rep_override, rep);
    let rep_rgba = if packed == REP_COLOR_INHERIT {
        base
    } else {
        unpack_rep_rgb8(packed)
    };
    let mut rgba = rep_rgba;

    let settings = input.object_settings.as_ref().unwrap_or(scene_settings);
    let atom = input
        .molecule
        .get_atom(AtomIndex(atom_id))
        .map(|atom| &atom.repr);

    let alpha_mul = match rep {
        RepKind::Sphere => rep_alpha(
            settings.sphere.transparency,
            atom.and_then(|r| r.sphere_transparency),
        ),
        RepKind::Stick => rep_alpha(
            settings.stick.transparency,
            atom.and_then(|r| r.stick_transparency),
        ),
        RepKind::Cartoon => rep_alpha(
            settings.cartoon.transparency,
            atom.and_then(|r| r.cartoon_transparency),
        ),
        RepKind::Surface => rep_alpha(
            settings.surface.transparency,
            atom.and_then(|r| r.surface_transparency),
        ),
        RepKind::Mesh => (1.0 - settings.mesh.transparency.clamp(0.0, 1.0)).clamp(0.0, 1.0),
        RepKind::Ribbon | RepKind::Line | RepKind::Dot => 1.0,
        RepKind::Ellipsoid => rep_alpha(
            settings.ellipsoid.transparency,
            atom.and_then(|r| r.ellipsoid_transparency),
        ),
        _ => 1.0,
    };
    rgba[3] = (rgba[3] * alpha_mul).clamp(0.0, 1.0);
    DisplayedMaterial {
        base_rgba: base,
        rep_rgba,
        rgba,
        transparency: 1.0 - rgba[3],
    }
}

pub(crate) fn oct_decode(packed: u32) -> [f32; 3] {
    let lo = (packed & 0xFFFF) as u16 as i16;
    let hi = ((packed >> 16) & 0xFFFF) as u16 as i16;
    let mut nx = (lo as f32 / 32767.0).max(-1.0);
    let mut ny = (hi as f32 / 32767.0).max(-1.0);
    let nz = 1.0 - nx.abs() - ny.abs();
    if nz < 0.0 {
        let tx = (1.0 - ny.abs()) * if nx >= 0.0 { 1.0 } else { -1.0 };
        let ty = (1.0 - nx.abs()) * if ny >= 0.0 { 1.0 } else { -1.0 };
        nx = tx;
        ny = ty;
    }
    normalize3([nx, ny, nz]).unwrap_or([0.0, 1.0, 0.0])
}

fn append_spheres(
    out: &mut DisplayedObjectGeometry,
    input: &RenderObjectInput<'_>,
    settings: &ResolvedSettings,
) {
    if !input.draw_reps.is_visible(RepMask::SPHERES) {
        return;
    }
    for (atom_id, coord) in input.coord_set.iter_with_atoms() {
        let Some(atom) = input.molecule.get_atom(atom_id) else {
            continue;
        };
        if !atom.repr.visible_reps.is_visible(RepMask::SPHERES) {
            continue;
        }
        let radius =
            atom.effective_vdw() * atom.repr.sphere_scale.unwrap_or(1.0) * settings.sphere.scale;
        out.primitives.push(DisplayedPrimitive::Sphere {
            rep: RepKind::Sphere,
            owner_atom_id: atom_id.as_u32(),
            center: [coord.x, coord.y, coord.z],
            radius,
            material: material_for_atom(input, settings, RepKind::Sphere, atom_id.as_u32()),
        });
    }
}

fn append_sticks(
    out: &mut DisplayedObjectGeometry,
    input: &RenderObjectInput<'_>,
    settings: &ResolvedSettings,
) {
    if !input.draw_reps.is_visible(RepMask::STICKS) {
        return;
    }
    let mut caps: HashMap<u32, (f32, DisplayedMaterial)> = HashMap::new();
    for bond in input.molecule.bonds() {
        let Some(atom_a) = input.molecule.get_atom(bond.atom1) else {
            continue;
        };
        let Some(atom_b) = input.molecule.get_atom(bond.atom2) else {
            continue;
        };
        if !atom_a.repr.visible_reps.is_visible(RepMask::STICKS)
            && !atom_b.repr.visible_reps.is_visible(RepMask::STICKS)
        {
            continue;
        }
        let Some(pa) = atom_position(input.coord_set, bond.atom1) else {
            continue;
        };
        let Some(pb) = atom_position(input.coord_set, bond.atom2) else {
            continue;
        };
        let Some(axis) = normalize3(sub3(pb, pa)) else {
            continue;
        };
        let offsets = if settings.stick.valence {
            bond_offsets(bond.order)
        } else {
            &[0.0]
        };
        let mut radius = settings.stick.radius;
        if offsets.len() > 1 {
            radius *= 0.5;
        }
        let valence_scale = settings.stick.radius * 1.5 * settings.stick.valence_scale;
        let perp = if offsets.len() > 1 {
            valence_perp(input.molecule, input.coord_set, bond.atom1, bond.atom2)
                .unwrap_or_else(|| fallback_perp(axis))
        } else {
            [0.0, 0.0, 0.0]
        };
        let mat_a = material_for_atom(input, settings, RepKind::Stick, bond.atom1.as_u32());
        let mat_b = material_for_atom(input, settings, RepKind::Stick, bond.atom2.as_u32());
        for &factor in offsets {
            let offset = mul3(perp, factor * valence_scale);
            out.primitives.push(DisplayedPrimitive::Cylinder {
                rep: RepKind::Stick,
                owner_atom_ids: [bond.atom1.as_u32(), bond.atom2.as_u32()],
                start: add3(pa, offset),
                end: add3(pb, offset),
                radius,
                material_start: mat_a,
                material_end: mat_b,
            });
        }
        update_cap(&mut caps, bond.atom1.as_u32(), radius, mat_a);
        update_cap(&mut caps, bond.atom2.as_u32(), radius, mat_b);
    }

    for (atom_id, (radius, material)) in caps {
        let Some(center) = atom_position(input.coord_set, AtomIndex(atom_id)) else {
            continue;
        };
        out.primitives.push(DisplayedPrimitive::Sphere {
            rep: RepKind::Stick,
            owner_atom_id: atom_id,
            center,
            radius,
            material,
        });
    }
}

fn append_lines(
    out: &mut DisplayedObjectGeometry,
    input: &RenderObjectInput<'_>,
    settings: &ResolvedSettings,
) {
    if !input.draw_reps.is_visible(RepMask::LINES) {
        return;
    }
    for bond in input.molecule.bonds() {
        let Some(atom_a) = input.molecule.get_atom(bond.atom1) else {
            continue;
        };
        let Some(atom_b) = input.molecule.get_atom(bond.atom2) else {
            continue;
        };
        if !atom_a.repr.visible_reps.is_visible(RepMask::LINES)
            && !atom_b.repr.visible_reps.is_visible(RepMask::LINES)
        {
            continue;
        }
        let Some(pa) = atom_position(input.coord_set, bond.atom1) else {
            continue;
        };
        let Some(pb) = atom_position(input.coord_set, bond.atom2) else {
            continue;
        };
        let offsets = if settings.stick.valence {
            bond_offsets(bond.order)
        } else {
            &[0.0]
        };
        let mut bond_dir = [0.0, 0.0, 0.0];
        let mut perp = [0.0, 0.0, 0.0];
        let valence_scale = settings.stick.radius * 1.5 * settings.stick.valence_scale;
        if offsets.len() > 1 {
            let Some(dir) = normalize3(sub3(pb, pa)) else {
                continue;
            };
            bond_dir = dir;
            perp = valence_perp(input.molecule, input.coord_set, bond.atom1, bond.atom2)
                .unwrap_or_else(|| fallback_perp(dir));
        }
        let mat_a = material_for_atom(input, settings, RepKind::Line, bond.atom1.as_u32());
        let mat_b = material_for_atom(input, settings, RepKind::Line, bond.atom2.as_u32());
        for &factor in offsets {
            let offset = mul3(perp, factor * valence_scale);
            let axis_join = mul3(
                bond_dir,
                (factor * valence_scale).abs() * VALENCE_JOIN_FACTOR,
            );
            out.primitives.push(DisplayedPrimitive::LineSegment {
                rep: RepKind::Line,
                owner_atom_ids: [bond.atom1.as_u32(), bond.atom2.as_u32()],
                start: sub3(add3(pa, offset), axis_join),
                end: add3(add3(pb, offset), axis_join),
                width_px: 1.49,
                material_start: mat_a,
                material_end: mat_b,
            });
        }
    }
}

fn append_dots(
    out: &mut DisplayedObjectGeometry,
    input: &RenderObjectInput<'_>,
    settings: &ResolvedSettings,
) {
    if !input.draw_reps.is_visible(RepMask::DOTS) {
        return;
    }
    let requested = requested_dots_per_atom_from_density(settings.dot.density);
    let dots_per_atom = effective_dots_per_atom(input.molecule.atom_count() as u32, requested);
    if dots_per_atom == 0 {
        return;
    }
    let radius_px = (settings.dot.width.max(0.5) * 0.5).max(0.25);
    let denom = (dots_per_atom as f32 - 1.0).max(1.0);
    for (atom_id, center) in input.coord_set.iter_with_atoms() {
        let Some(atom) = input.molecule.get_atom(atom_id) else {
            continue;
        };
        if !atom.repr.visible_reps.is_visible(RepMask::DOTS) {
            continue;
        }
        let center = [center.x, center.y, center.z];
        let radius = atom.effective_vdw() * atom.repr.sphere_scale.unwrap_or(1.0);
        let material = material_for_atom(input, settings, RepKind::Dot, atom_id.as_u32());
        for slot in 0..dots_per_atom {
            let i = slot as f32;
            let y = 1.0 - (i / denom) * 2.0;
            let r = (1.0 - y * y).max(0.0).sqrt();
            let theta = GOLDEN_ANGLE * i;
            let sample = [theta.cos() * r, y, theta.sin() * r];
            out.primitives.push(DisplayedPrimitive::PointSample {
                rep: RepKind::Dot,
                owner_atom_id: atom_id.as_u32(),
                position: add3(center, mul3(sample, radius)),
                radius_px,
                material,
            });
        }
    }
}

fn rep_packed_color(entry: RepColorLutEntry, rep: RepKind) -> u32 {
    match rep {
        RepKind::Sphere => entry.sphere,
        RepKind::Stick => entry.stick,
        RepKind::Line => entry.line,
        RepKind::Dot => entry.dot,
        RepKind::Cartoon => entry.cartoon,
        RepKind::Ribbon => entry.ribbon,
        RepKind::Surface => entry.surface,
        RepKind::Mesh => entry.mesh,
        RepKind::Ellipsoid => entry.ellipsoid,
        _ => REP_COLOR_INHERIT,
    }
}

fn unpack_rep_rgb8(packed: u32) -> [f32; 4] {
    [
        (packed & 0xFF) as f32 / 255.0,
        ((packed >> 8) & 0xFF) as f32 / 255.0,
        ((packed >> 16) & 0xFF) as f32 / 255.0,
        1.0,
    ]
}

fn rep_alpha(fallback_transparency: f32, override_transparency: Option<f32>) -> f32 {
    match override_transparency {
        Some(t) => {
            let alpha = (1.0 - t.clamp(0.0, 1.0)).clamp(0.0, 1.0);
            (alpha * 254.0).round().clamp(0.0, 254.0) / 254.0
        }
        None => (1.0 - fallback_transparency.clamp(0.0, 1.0)).clamp(0.0, 1.0),
    }
}

fn requested_dots_per_atom_from_density(density: i32) -> u32 {
    let density = density.clamp(0, 4) as u32;
    16u32.saturating_mul(1u32 << density)
}

fn effective_dots_per_atom(atom_count: u32, requested: u32) -> u32 {
    if atom_count == 0 {
        return requested;
    }
    let cap_per_atom = MAX_DOT_INSTANCES / atom_count as u64;
    let cap = cap_per_atom.max(1) as u32;
    requested.min(cap)
}

fn bond_offsets(order: BondOrder) -> &'static [f32] {
    match order {
        BondOrder::Double => &[-0.5, 0.5],
        BondOrder::Triple => &[-1.0, 0.0, 1.0],
        BondOrder::Aromatic => &[-0.5, 0.5],
        _ => &[0.0],
    }
}

fn atom_position(coord_set: &CoordSet, atom_id: AtomIndex) -> Option<[f32; 3]> {
    let p = coord_set.get_atom_coord(atom_id)?;
    Some([p.x, p.y, p.z])
}

fn update_cap(
    caps: &mut HashMap<u32, (f32, DisplayedMaterial)>,
    atom_id: u32,
    radius: f32,
    material: DisplayedMaterial,
) {
    caps.entry(atom_id)
        .and_modify(|(current, current_material)| {
            if radius > *current {
                *current = radius;
                *current_material = material;
            }
        })
        .or_insert((radius, material));
}

fn valence_perp(
    molecule: &ObjectMolecule,
    coord_set: &CoordSet,
    atom_a: AtomIndex,
    atom_b: AtomIndex,
) -> Option<[f32; 3]> {
    let pa = atom_position(coord_set, atom_a)?;
    let pb = atom_position(coord_set, atom_b)?;
    let bond_dir = normalize3(sub3(pb, pa))?;
    let mid = mul3(add3(pa, pb), 0.5);

    let mut best: Option<[f32; 3]> = None;
    for endpoint in [atom_a, atom_b] {
        for nb in molecule.bonded_atoms_iter(endpoint) {
            if nb == atom_a || nb == atom_b {
                continue;
            }
            let Some(atom) = molecule.get_atom(nb) else {
                continue;
            };
            if !atom.is_heavy() {
                continue;
            }
            let Some(pos) = atom_position(coord_set, nb) else {
                continue;
            };
            let to_neighbor = sub3(pos, mid);
            let projected = sub3(to_neighbor, mul3(bond_dir, dot3(to_neighbor, bond_dir)));
            if let Some(candidate) = normalize3(projected) {
                best = Some(candidate);
                break;
            }
        }
        if best.is_some() {
            break;
        }
    }
    best
}

fn fallback_perp(bond_dir: [f32; 3]) -> [f32; 3] {
    normalize3(cross3(bond_dir, [0.0, 1.0, 0.0]))
        .or_else(|| normalize3(cross3(bond_dir, [1.0, 0.0, 0.0])))
        .unwrap_or([1.0, 0.0, 0.0])
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul3(v: [f32; 3], scale: f32) -> [f32; 3] {
    [v[0] * scale, v[1] * scale, v[2] * scale]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize3(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = dot3(v, v).sqrt();
    if len > 1e-6 {
        Some([v[0] / len, v[1] / len, v[2] / len])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, BondOrder, CoordSet, Element, ObjectMolecule};
    use patinae_settings::Settings;

    fn test_object(reps: RepMask) -> (ObjectMolecule, CoordSet) {
        let mut mol = ObjectMolecule::new("geom");
        let mut a = Atom::new("C1", Element::Carbon);
        a.repr.visible_reps = reps;
        let mut b = Atom::new("O1", Element::Oxygen);
        b.repr.visible_reps = reps;
        let a_id = mol.add_atom(a);
        let b_id = mol.add_atom(b);
        mol.add_bond(a_id, b_id, BondOrder::Single).unwrap();
        let coords = CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.5, 0.0, 0.0)]);
        (mol, coords)
    }

    fn input_for<'a>(
        mol: &'a ObjectMolecule,
        coord_set: &'a CoordSet,
        colors: &'a [[f32; 4]],
        rep_colors: &'a [RepColorLutEntry],
        reps: RepMask,
    ) -> RenderObjectInput<'a> {
        RenderObjectInput {
            object_id: ObjectId(1),
            molecule: mol,
            coord_set,
            transform: crate::render_input::IDENTITY_TRANSFORM,
            visible_reps: reps,
            draw_reps: reps,
            object_settings: None,
            atom_colors: colors,
            atom_rep_colors: rep_colors,
            atom_markers: &[],
            marker_updates: &[],
            has_markers: false,
            lod: crate::render_input::SceneLod::High,
            dirty: patinae_mol::DirtyFlags::ALL,
        }
    }

    #[test]
    fn hidden_object_exports_no_cpu_geometry() {
        let (mol, coords) = test_object(RepMask::SPHERES);
        let colors = vec![[1.0, 0.0, 0.0, 1.0]; mol.atom_count()];
        let rep_colors = vec![RepColorLutEntry::default(); mol.atom_count()];
        let resolved = ResolvedSettings::resolve(&Settings::default(), None);
        let input = input_for(&mol, &coords, &colors, &rep_colors, RepMask::NONE);
        let mut obj = DisplayedObjectGeometry {
            object_id: ObjectId(1),
            primitives: Vec::new(),
        };

        append_cpu_object_geometry(
            &mut obj,
            &input,
            &resolved,
            &GeometryExportOptions::default(),
        );

        assert!(obj.primitives.is_empty());
    }

    #[test]
    fn cpu_export_covers_analytic_and_semantic_reps() {
        let reps = RepMask::SPHERES
            .union(RepMask::STICKS)
            .union(RepMask::LINES)
            .union(RepMask::DOTS);
        let (mol, coords) = test_object(reps);
        let colors = vec![[1.0, 0.0, 0.0, 1.0]; mol.atom_count()];
        let rep_colors = vec![RepColorLutEntry::default(); mol.atom_count()];
        let resolved = ResolvedSettings::resolve(&Settings::default(), None);
        let input = input_for(&mol, &coords, &colors, &rep_colors, reps);
        let mut obj = DisplayedObjectGeometry {
            object_id: ObjectId(1),
            primitives: Vec::new(),
        };

        append_cpu_object_geometry(
            &mut obj,
            &input,
            &resolved,
            &GeometryExportOptions::default(),
        );

        assert!(obj
            .primitives
            .iter()
            .any(|p| p.rep_kind() == RepKind::Sphere));
        assert!(obj
            .primitives
            .iter()
            .any(|p| p.rep_kind() == RepKind::Stick));
        assert!(obj.primitives.iter().any(|p| p.rep_kind() == RepKind::Line));
        assert!(obj.primitives.iter().any(|p| p.rep_kind() == RepKind::Dot));
    }

    #[test]
    fn sphere_export_multiplies_atom_and_rep_scale() {
        let (mut mol, coords) = test_object(RepMask::SPHERES);
        mol.get_atom_mut(AtomIndex(0)).unwrap().repr.sphere_scale = Some(2.0);
        let colors = vec![[1.0, 1.0, 1.0, 1.0]; mol.atom_count()];
        let rep_colors = vec![RepColorLutEntry::default(); mol.atom_count()];
        let mut settings = Settings::default();
        settings.sphere.scale = 1.5;
        let resolved = ResolvedSettings::resolve(&settings, None);
        let input = input_for(&mol, &coords, &colors, &rep_colors, RepMask::SPHERES);
        let mut obj = DisplayedObjectGeometry {
            object_id: ObjectId(1),
            primitives: Vec::new(),
        };

        append_cpu_object_geometry(
            &mut obj,
            &input,
            &resolved,
            &GeometryExportOptions::default(),
        );

        let radius = obj
            .primitives
            .iter()
            .find_map(|p| match p {
                DisplayedPrimitive::Sphere {
                    owner_atom_id: 0,
                    radius,
                    ..
                } => Some(*radius),
                _ => None,
            })
            .unwrap();
        let expected = mol.get_atom(AtomIndex(0)).unwrap().effective_vdw() * 2.0 * 1.5;
        assert!((radius - expected).abs() < 1e-5);
    }

    #[test]
    fn object_transform_moves_exported_geometry_into_world_space() {
        let (mol, coords) = test_object(RepMask::SPHERES);
        let colors = vec![[1.0, 1.0, 1.0, 1.0]; mol.atom_count()];
        let rep_colors = vec![RepColorLutEntry::default(); mol.atom_count()];
        let resolved = ResolvedSettings::resolve(&Settings::default(), None);
        let mut input = input_for(&mol, &coords, &colors, &rep_colors, RepMask::SPHERES);
        input.transform[3] = [2.0, 3.0, 4.0, 1.0];
        let mut object = DisplayedObjectGeometry {
            object_id: ObjectId(1),
            primitives: Vec::new(),
        };

        append_cpu_object_geometry(
            &mut object,
            &input,
            &resolved,
            &GeometryExportOptions::default(),
        );

        let center = object
            .primitives
            .iter()
            .find_map(|primitive| match primitive {
                DisplayedPrimitive::Sphere {
                    owner_atom_id: 0,
                    center,
                    ..
                } => Some(*center),
                _ => None,
            })
            .expect("first sphere");
        assert_eq!(center, [2.0, 3.0, 4.0]);
    }
}
