//! Integrated nucleotide rung and base-plate geometry for cartoons.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use lin_alg::f32::Vec3;
use patinae_mol::{CoordSet, ObjectMolecule, ResidueView, SecondaryStructure};

use super::backbone::{flags, BackboneAtom};
use crate::representations::mesh::{oct_encode, StdVertex};

/// The connector stays subordinate to the cartoon while remaining visible.
const RUNG_RADIUS: f32 = 0.15;
/// Eight sides keep the connector compact and bound its per-residue GPU cost.
const RUNG_SIDES: usize = 8;
/// Canonical half-length of the abstract base plate, in angstroms.
const PLATE_HALF_LENGTH: f32 = 1.25;
/// Canonical half-width of the abstract base plate, in angstroms.
const PLATE_HALF_WIDTH: f32 = 0.9;
/// Small non-zero thickness gives the base plate stable lighting and shadows.
const PLATE_HALF_THICKNESS: f32 = 0.08;
/// Connectors shorter than this are invisible and would emit zero-area triangles.
const MIN_RUNG_LENGTH_SQUARED: f32 = 1.0e-8;
const CYLINDER_VERTICES: u64 = (RUNG_SIDES as u64) * 12;
const PLATE_VERTICES: u64 = 36;

pub(super) const MAX_VERTICES_PER_NUCLEOTIDE: u64 = CYLINDER_VERTICES + PLATE_VERTICES;

#[derive(Debug)]
pub(super) struct NucleicGeometryPrep {
    residues: Vec<NucleotidePlacement>,
    pub(super) signature: u64,
}

impl NucleicGeometryPrep {
    pub(super) fn emit_vertices(&self) -> Vec<StdVertex> {
        let mut vertices = Vec::with_capacity(
            self.residues
                .len()
                .saturating_mul(MAX_VERTICES_PER_NUCLEOTIDE as usize),
        );
        for residue in &self.residues {
            if (residue.plate.rung_end - residue.anchor).magnitude_squared()
                > MIN_RUNG_LENGTH_SQUARED
            {
                append_rung(
                    &mut vertices,
                    residue.anchor,
                    residue.plate.rung_end,
                    residue.plate.long,
                    residue.plate.normal,
                    residue.group_id,
                );
            }
            append_plate(&mut vertices, residue.plate, residue.group_id);
        }
        vertices
    }
}

#[derive(Debug, Clone, Copy)]
struct NucleotidePlacement {
    anchor: Vec3,
    plate: PlatePlacement,
    group_id: u32,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlateFrameSource {
    Structure,
    Fallback,
}

#[derive(Debug, Clone, Copy)]
struct PlatePlacement {
    center: Vec3,
    long: Vec3,
    width: Vec3,
    normal: Vec3,
    rung_end: Vec3,
    #[cfg(test)]
    source: PlateFrameSource,
}

fn is_nucleic_backbone_atom_name(name: &str) -> bool {
    matches!(
        name,
        "P" | "OP1"
            | "OP2"
            | "OP3"
            | "O1P"
            | "O2P"
            | "O3P"
            | "O5'"
            | "C5'"
            | "C4'"
            | "O4'"
            | "C3'"
            | "O3'"
            | "C2'"
            | "O2'"
            | "C1'"
    )
}

fn normalized_or(value: Vec3, fallback: Vec3) -> Vec3 {
    let length = value.magnitude();
    if length > 1.0e-6 {
        value / length
    } else {
        fallback
    }
}

fn perpendicular_to(axis: Vec3) -> Vec3 {
    let reference = if axis.y.abs() < 0.9 {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    normalized_or(
        reference - axis * reference.dot(axis),
        Vec3::new(0.0, 0.0, 1.0),
    )
}

fn fallback_frame(orientation: Vec3, tangent: Vec3) -> (Vec3, Vec3, Vec3) {
    let long = normalized_or(orientation, Vec3::new(1.0, 0.0, 0.0));
    let tangent = normalized_or(tangent, perpendicular_to(long));
    let normal = normalized_or(long.cross(tangent), perpendicular_to(long));
    let width = normalized_or(normal.cross(long), perpendicular_to(long));
    (long, width, normal)
}

fn structure_frame(
    base_positions: &[Vec3],
    center: Vec3,
    direction_hint: Vec3,
) -> Option<(Vec3, Vec3, Vec3)> {
    if base_positions.len() < 3 {
        return None;
    }

    let primary = base_positions
        .iter()
        .map(|point| *point - center)
        .max_by(|left, right| {
            left.magnitude_squared()
                .total_cmp(&right.magnitude_squared())
        })?;
    let secondary = base_positions
        .iter()
        .map(|point| *point - center)
        .max_by(|left, right| {
            primary
                .cross(*left)
                .magnitude_squared()
                .total_cmp(&primary.cross(*right).magnitude_squared())
        })?;
    let normal = normalized_or(primary.cross(secondary), Vec3::new(0.0, 0.0, 0.0));
    if normal.magnitude_squared() < 1.0e-8 {
        return None;
    }

    let projected_hint = direction_hint - normal * direction_hint.dot(normal);
    let long = normalized_or(
        projected_hint,
        normalized_or(primary, perpendicular_to(normal)),
    );
    let width = normalized_or(normal.cross(long), perpendicular_to(long));
    Some((long, width, normal))
}

fn base_plate_placement(
    base_positions: &[Vec3],
    anchor: Vec3,
    c1: Option<Vec3>,
    orientation: Vec3,
    tangent: Vec3,
) -> Option<PlatePlacement> {
    let (fallback_long, fallback_width, fallback_normal) = fallback_frame(orientation, tangent);
    if base_positions.len() >= 3 {
        let center = base_positions
            .iter()
            .copied()
            .fold(Vec3::new(0.0, 0.0, 0.0), |sum, point| sum + point)
            / base_positions.len() as f32;
        let origin = c1.unwrap_or(anchor);
        if let Some((long, width, mut normal)) =
            structure_frame(base_positions, center, center - origin)
        {
            if normal.dot(fallback_normal) < 0.0 {
                normal = -normal;
            }
            let width = if normal.dot(long.cross(width)) < 0.0 {
                -width
            } else {
                width
            };
            return Some(PlatePlacement {
                center,
                long,
                width,
                normal,
                rung_end: center - long * PLATE_HALF_LENGTH,
                #[cfg(test)]
                source: PlateFrameSource::Structure,
            });
        }
    }

    let c1 = c1?;
    let center = c1 + fallback_long * PLATE_HALF_LENGTH;
    Some(PlatePlacement {
        center,
        long: fallback_long,
        width: fallback_width,
        normal: fallback_normal,
        rung_end: c1,
        #[cfg(test)]
        source: PlateFrameSource::Fallback,
    })
}

fn push_vertex(vertices: &mut Vec<StdVertex>, position: Vec3, normal: Vec3, group_id: u32) {
    vertices.push(StdVertex {
        position: [position.x, position.y, position.z],
        normal_oct: oct_encode([normal.x, normal.y, normal.z]),
        group_id,
        flags: 0,
    });
}

fn push_triangle(vertices: &mut Vec<StdVertex>, points: [Vec3; 3], normal: Vec3, group_id: u32) {
    for point in points {
        push_vertex(vertices, point, normal, group_id);
    }
}

fn append_rung(
    vertices: &mut Vec<StdVertex>,
    start: Vec3,
    end: Vec3,
    axis_hint: Vec3,
    normal_hint: Vec3,
    group_id: u32,
) {
    let axis = normalized_or(
        end - start,
        normalized_or(axis_hint, Vec3::new(1.0, 0.0, 0.0)),
    );
    let radial_u = normalized_or(
        normal_hint - axis * normal_hint.dot(axis),
        perpendicular_to(axis),
    );
    let radial_v = normalized_or(axis.cross(radial_u), perpendicular_to(axis));
    let ring: [Vec3; RUNG_SIDES] = std::array::from_fn(|side| {
        let angle = std::f32::consts::TAU * side as f32 / RUNG_SIDES as f32;
        radial_u * angle.cos() + radial_v * angle.sin()
    });

    for side in 0..RUNG_SIDES {
        let next = (side + 1) % RUNG_SIDES;
        let normal_a = ring[side];
        let normal_b = ring[next];
        let start_a = start + normal_a * RUNG_RADIUS;
        let start_b = start + normal_b * RUNG_RADIUS;
        let end_a = end + normal_a * RUNG_RADIUS;
        let end_b = end + normal_b * RUNG_RADIUS;

        push_triangle(
            vertices,
            [start_a, end_b, end_a],
            normalized_or(normal_a + normal_b, normal_a),
            group_id,
        );
        push_triangle(
            vertices,
            [start_a, start_b, end_b],
            normalized_or(normal_a + normal_b, normal_a),
            group_id,
        );
        push_triangle(vertices, [start, start_b, start_a], -axis, group_id);
        push_triangle(vertices, [end, end_a, end_b], axis, group_id);
    }
}

fn append_quad(vertices: &mut Vec<StdVertex>, corners: [Vec3; 4], normal: Vec3, group_id: u32) {
    push_triangle(
        vertices,
        [corners[0], corners[1], corners[2]],
        normal,
        group_id,
    );
    push_triangle(
        vertices,
        [corners[0], corners[2], corners[3]],
        normal,
        group_id,
    );
}

fn append_plate(vertices: &mut Vec<StdVertex>, plate: PlatePlacement, group_id: u32) {
    let x = plate.long * PLATE_HALF_LENGTH;
    let y = plate.width * PLATE_HALF_WIDTH;
    let z = plate.normal * PLATE_HALF_THICKNESS;
    let c = plate.center;
    let x0y0z0 = c - x - y - z;
    let x0y0z1 = c - x - y + z;
    let x0y1z0 = c - x + y - z;
    let x0y1z1 = c - x + y + z;
    let x1y0z0 = c + x - y - z;
    let x1y0z1 = c + x - y + z;
    let x1y1z0 = c + x + y - z;
    let x1y1z1 = c + x + y + z;

    append_quad(
        vertices,
        [x0y0z1, x1y0z1, x1y1z1, x0y1z1],
        plate.normal,
        group_id,
    );
    append_quad(
        vertices,
        [x0y1z0, x1y1z0, x1y0z0, x0y0z0],
        -plate.normal,
        group_id,
    );
    append_quad(
        vertices,
        [x1y0z0, x1y1z0, x1y1z1, x1y0z1],
        plate.long,
        group_id,
    );
    append_quad(
        vertices,
        [x0y0z1, x0y1z1, x0y1z0, x0y0z0],
        -plate.long,
        group_id,
    );
    append_quad(
        vertices,
        [x0y1z0, x0y1z1, x1y1z1, x1y1z0],
        plate.width,
        group_id,
    );
    append_quad(
        vertices,
        [x0y0z1, x0y0z0, x1y0z0, x1y0z1],
        -plate.width,
        group_id,
    );
}

fn backbone_tangent(backbone: &[BackboneAtom], index: usize) -> Vec3 {
    let position =
        |item: &BackboneAtom| Vec3::new(item.position[0], item.position[1], item.position[2]);
    let has_previous = index > 0 && (backbone[index - 1].flags & flags::SEG_END) == 0;
    let has_next = index + 1 < backbone.len() && (backbone[index].flags & flags::SEG_END) == 0;
    match (has_previous, has_next) {
        (true, true) => position(&backbone[index + 1]) - position(&backbone[index - 1]),
        (true, false) => position(&backbone[index]) - position(&backbone[index - 1]),
        (false, true) => position(&backbone[index + 1]) - position(&backbone[index]),
        (false, false) => perpendicular_to(Vec3::new(
            backbone[index].orientation[0],
            backbone[index].orientation[1],
            backbone[index].orientation[2],
        )),
    }
}

fn residue_placement(
    residue: &ResidueView<'_>,
    coord_set: &CoordSet,
    backbone: &BackboneAtom,
    tangent: Vec3,
    base_positions: &mut Vec<Vec3>,
) -> Option<NucleotidePlacement> {
    let anchor = Vec3::new(
        backbone.position[0],
        backbone.position[1],
        backbone.position[2],
    );
    let orientation = Vec3::new(
        backbone.orientation[0],
        backbone.orientation[1],
        backbone.orientation[2],
    );
    let c1 = residue
        .find_by_name("C1'")
        .and_then(|(index, _)| coord_set.get_atom_coord(index));
    base_positions.clear();
    base_positions.extend(
        residue
            .iter_indexed()
            .filter(|(_, atom)| !atom.is_hydrogen() && !is_nucleic_backbone_atom_name(&atom.name))
            .filter_map(|(index, _)| coord_set.get_atom_coord(index)),
    );
    let plate = base_plate_placement(base_positions, anchor, c1, orientation, tangent)?;
    Some(NucleotidePlacement {
        anchor,
        plate,
        group_id: backbone.atom_id,
    })
}

pub(super) fn count_nucleic_residues(backbone: &[BackboneAtom]) -> usize {
    backbone
        .iter()
        .filter(|atom| atom.flags & flags::SS_MASK == SecondaryStructure::NucleicRibbon as u32)
        .count()
}

fn hash_vec3(value: Vec3, hasher: &mut impl Hasher) {
    value.x.to_bits().hash(hasher);
    value.y.to_bits().hash(hasher);
    value.z.to_bits().hash(hasher);
}

pub(super) fn prepare_nucleic_geometry(
    molecule: &ObjectMolecule,
    coord_set: &CoordSet,
    backbone: &[BackboneAtom],
    enabled: bool,
) -> NucleicGeometryPrep {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    enabled.hash(&mut hasher);
    if !enabled {
        return NucleicGeometryPrep {
            residues: Vec::new(),
            signature: hasher.finish(),
        };
    }

    let guide_indices: HashMap<u32, usize> = backbone
        .iter()
        .enumerate()
        .filter(|(_, atom)| atom.flags & flags::SS_MASK == SecondaryStructure::NucleicRibbon as u32)
        .map(|(index, atom)| (atom.atom_id, index))
        .collect();
    let mut residues = Vec::with_capacity(guide_indices.len());
    let mut base_positions = Vec::new();

    for residue in molecule.residues().filter(ResidueView::is_nucleic) {
        let Some(backbone_index) = residue
            .iter_indexed()
            .find_map(|(atom_index, _)| guide_indices.get(&atom_index.as_u32()).copied())
        else {
            continue;
        };
        let tangent = backbone_tangent(backbone, backbone_index);
        if let Some(placement) = residue_placement(
            &residue,
            coord_set,
            &backbone[backbone_index],
            tangent,
            &mut base_positions,
        ) {
            placement.group_id.hash(&mut hasher);
            hash_vec3(placement.anchor, &mut hasher);
            hash_vec3(placement.plate.center, &mut hasher);
            hash_vec3(placement.plate.long, &mut hasher);
            hash_vec3(placement.plate.width, &mut hasher);
            hash_vec3(placement.plate.normal, &mut hasher);
            hash_vec3(placement.plate.rung_end, &mut hasher);
            residues.push(placement);
        }
    }

    residues.len().hash(&mut hasher);
    NucleicGeometryPrep {
        residues,
        signature: hasher.finish(),
    }
}

#[cfg(test)]
#[derive(Debug)]
struct NucleicGeometry {
    vertices: Vec<StdVertex>,
    signature: u64,
}

#[cfg(test)]
fn build_nucleic_geometry(
    molecule: &ObjectMolecule,
    coord_set: &CoordSet,
    backbone: &[BackboneAtom],
    enabled: bool,
) -> NucleicGeometry {
    let prep = prepare_nucleic_geometry(molecule, coord_set, backbone, enabled);
    NucleicGeometry {
        vertices: prep.emit_vertices(),
        signature: prep.signature,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry_export::analytic::oct_decode;
    use crate::representations::cartoon::backbone::BackboneAtom;
    use lin_alg::f32::Vec3;
    use patinae_mol::{
        AtomBuilder, AtomFlags, AtomIndex, AtomResidue, MoleculeBuilder, ObjectMolecule, RepMask,
        SecondaryStructure,
    };
    use std::sync::Arc;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).magnitude() < 1.0e-4
    }

    fn synthetic_nucleotides(count: usize) -> (ObjectMolecule, Vec<BackboneAtom>) {
        const NUCLEOTIDE_SPACING: f32 = 6.0;

        let mut builder = MoleculeBuilder::new("nt");
        for residue_index in 0..count {
            let residue_number =
                i32::try_from(residue_index).expect("synthetic residue index fits i32") + 1;
            let residue = Arc::new(AtomResidue::from_parts("A", "DA", residue_number, ' ', ""));
            let offset = Vec3::new(0.0, residue_index as f32 * NUCLEOTIDE_SPACING, 0.0);
            for (name, element, position) in [
                ("P", "P", Vec3::new(0.0, 0.0, 0.0)),
                ("C1'", "C", Vec3::new(1.5, 0.0, 0.0)),
                ("N9", "N", Vec3::new(2.2, 0.0, 0.0)),
                ("C8", "C", Vec3::new(3.0, -0.9, 0.0)),
                ("N7", "N", Vec3::new(4.0, -0.5, 0.0)),
                ("C5", "C", Vec3::new(4.0, 0.6, 0.0)),
                ("C6", "C", Vec3::new(3.0, 1.0, 0.0)),
            ] {
                let mut atom = AtomBuilder::new()
                    .name(name)
                    .element_symbol(element)
                    .build();
                atom.residue = residue.clone();
                atom.state.flags |= AtomFlags::NUCLEIC | AtomFlags::POLYMER;
                builder = builder.add_atom(atom, position + offset);
            }
        }
        let molecule = builder.build();
        let coord_set = molecule.current_coord_set().expect("coordinate set");
        let backbone = molecule
            .atoms_indexed()
            .filter(|(_, atom)| &*atom.name == "P")
            .map(|(index, _)| {
                let position = coord_set.get_atom_coord(index).expect("P coordinate");
                BackboneAtom {
                    position: [position.x, position.y, position.z],
                    atom_id: index.as_u32(),
                    orientation: [1.0, 0.0, 0.0],
                    flags: SecondaryStructure::NucleicRibbon as u32,
                }
            })
            .collect();

        (molecule, backbone)
    }

    fn one_nucleotide() -> (ObjectMolecule, BackboneAtom, AtomIndex) {
        let (molecule, mut backbone) = synthetic_nucleotides(1);
        let guide_id = molecule.atoms_indexed().next().expect("P atom").0;
        let base_id = molecule
            .atoms_indexed()
            .find(|(_, atom)| &*atom.name == "C8")
            .expect("C8 atom")
            .0;
        let backbone = backbone.pop().expect("synthetic backbone atom");
        assert_eq!(backbone.atom_id, guide_id.as_u32());
        (molecule, backbone, base_id)
    }

    #[test]
    fn structure_frame_uses_base_centroid_and_plane() {
        let base = [
            Vec3::new(2.0, -1.0, 0.0),
            Vec3::new(4.0, -1.0, 0.0),
            Vec3::new(4.0, 1.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
        ];
        let placement = base_plate_placement(
            &base,
            Vec3::new(0.0, 0.0, 0.0),
            Some(Vec3::new(1.0, 0.0, 0.0)),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
        .expect("structure-derived placement");

        assert_eq!(placement.source, PlateFrameSource::Structure);
        assert!(close(placement.center, Vec3::new(3.0, 0.0, 0.0)));
        assert!(placement.normal.z.abs() > 0.999);
        assert!((placement.rung_end - placement.center).magnitude() < 2.0);
    }

    #[test]
    fn sparse_residue_falls_back_to_c1_and_cartoon_frame() {
        let placement = base_plate_placement(
            &[],
            Vec3::new(0.0, 0.0, 0.0),
            Some(Vec3::new(1.0, 0.0, 0.0)),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
        .expect("fallback placement");

        assert_eq!(placement.source, PlateFrameSource::Fallback);
        assert!(placement.center.x > 1.0);
        assert!(placement.normal.z.abs() > 0.999);
    }

    #[test]
    fn c1_backbone_fallback_skips_zero_length_connector() {
        let residue = Arc::new(AtomResidue::from_parts("A", "DA", 1, ' ', ""));
        let mut c1 = AtomBuilder::new().name("C1'").element_symbol("C").build();
        c1.residue = residue;
        c1.state.flags |= AtomFlags::NUCLEIC | AtomFlags::POLYMER;
        let molecule = MoleculeBuilder::new("sparse")
            .add_atom(c1, Vec3::new(1.0, 0.0, 0.0))
            .build();
        let guide_id = molecule.atoms_indexed().next().expect("C1 atom").0;
        let backbone = BackboneAtom {
            position: [1.0, 0.0, 0.0],
            atom_id: guide_id.as_u32(),
            orientation: [1.0, 0.0, 0.0],
            flags: SecondaryStructure::NucleicRibbon as u32,
        };
        let geometry = build_nucleic_geometry(
            &molecule,
            molecule.current_coord_set().expect("coord set"),
            &[backbone],
            true,
        );

        assert_eq!(geometry.vertices.len(), PLATE_VERTICES as usize);
    }

    #[test]
    fn emits_connected_cartoon_owned_geometry_with_guide_identity() {
        let (molecule, backbone, _) = one_nucleotide();
        assert!(molecule
            .atoms()
            .all(|atom| !atom.repr.visible_reps.is_visible(RepMask::STICKS)));
        let coord_set = molecule.current_coord_set().expect("coord set");
        let geometry = build_nucleic_geometry(&molecule, coord_set, &[backbone], true);

        assert_eq!(
            geometry.vertices.len(),
            MAX_VERTICES_PER_NUCLEOTIDE as usize
        );
        assert!(geometry
            .vertices
            .iter()
            .all(|vertex| vertex.group_id == backbone.atom_id));
        assert!(geometry.vertices.iter().all(|vertex| {
            vertex
                .position
                .iter()
                .all(|component| component.is_finite())
        }));
        assert!(geometry.vertices.iter().any(|vertex| {
            close(
                Vec3::new(vertex.position[0], vertex.position[1], vertex.position[2]),
                Vec3::new(0.0, 0.0, 0.0),
            )
        }));
        for triangle in geometry.vertices.as_chunks::<3>().0 {
            let point = |vertex: &StdVertex| {
                Vec3::new(vertex.position[0], vertex.position[1], vertex.position[2])
            };
            let geometric_normal = (point(&triangle[1]) - point(&triangle[0]))
                .cross(point(&triangle[2]) - point(&triangle[0]));
            assert!(geometric_normal.magnitude_squared() > 1.0e-8);
            let encoded = oct_decode(triangle[0].normal_oct);
            let encoded_normal = Vec3::new(encoded[0], encoded[1], encoded[2]);
            assert!(
                geometric_normal.dot(encoded_normal) > 0.0,
                "triangle winding must agree with its encoded normal"
            );
        }
    }

    #[test]
    fn setting_disables_geometry_and_changes_signature() {
        let (molecule, backbone, _) = one_nucleotide();
        let coord_set = molecule.current_coord_set().expect("coord set");
        let enabled = build_nucleic_geometry(&molecule, coord_set, &[backbone], true);
        let disabled = build_nucleic_geometry(&molecule, coord_set, &[backbone], false);

        assert!(!enabled.vertices.is_empty());
        assert!(disabled.vertices.is_empty());
        assert_ne!(enabled.signature, disabled.signature);
    }

    #[test]
    fn base_coordinate_changes_geometry_signature() {
        let (molecule, backbone, base_id) = one_nucleotide();
        let coord_set = molecule.current_coord_set().expect("coord set");
        let before = build_nucleic_geometry(&molecule, coord_set, &[backbone], true);
        let mut changed = coord_set.clone();
        assert!(changed.set_atom_coord(base_id, Vec3::new(3.0, -0.9, 0.5)));
        let after = build_nucleic_geometry(&molecule, &changed, &[backbone], true);

        assert_ne!(before.signature, after.signature);
    }

    #[test]
    fn synthetic_nucleotides_emit_one_integrated_accent_each() {
        const NUCLEOTIDE_COUNT: usize = 3;

        let (molecule, backbone) = synthetic_nucleotides(NUCLEOTIDE_COUNT);
        let coord_set = molecule.current_coord_set().expect("coord set");
        let nucleotide_count = count_nucleic_residues(&backbone);
        let geometry = build_nucleic_geometry(&molecule, coord_set, &backbone, true);

        assert_eq!(nucleotide_count, NUCLEOTIDE_COUNT);
        assert_eq!(
            geometry.vertices.len(),
            nucleotide_count * MAX_VERTICES_PER_NUCLEOTIDE as usize
        );
    }
}
