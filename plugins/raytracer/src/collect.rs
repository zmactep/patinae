//! Primitive collection from molecular data
//!
//! This module provides functions to extract raytracing primitives from
//! molecular structures, similar to how the render representations work.

use std::collections::HashMap;

use patinae_mol::{AtomIndex, CoordSet, ObjectMolecule, RepMask};
use patinae_scene::{ColorResolver, RepColorDefaults};

use crate::primitive::{GpuCylinder, GpuSphere, PrimitiveCollector, Primitives};

/// Collect sphere primitives from a molecule
///
/// Extracts atoms visible as spheres representation.
pub fn collect_spheres(
    molecule: &ObjectMolecule,
    coord_set: &CoordSet,
    colors: &ColorResolver<'_>,
    defaults: &RepColorDefaults,
    sphere_scale: f32,
    sphere_transparency: f32,
) -> Vec<GpuSphere> {
    let mut spheres = Vec::new();

    for (atom_idx, coord) in coord_set.iter_with_atoms() {
        let atom = match molecule.get_atom(atom_idx) {
            Some(a) => a,
            None => continue,
        };

        // Check if spheres representation is visible
        if !atom.repr.visible_reps.is_visible(RepMask::SPHERES) {
            continue;
        }

        // Use per-atom sphere_scale if set, otherwise global
        let scale = atom.repr.sphere_scale.unwrap_or(sphere_scale);
        let radius = atom.effective_vdw() * scale;
        let color = colors.resolve_rep_color(atom, atom.repr.colors.sphere, defaults.sphere);

        // Use per-atom transparency if set, otherwise fall back to global
        let transparency = atom.repr.sphere_transparency.unwrap_or(sphere_transparency);

        spheres.push(GpuSphere::new(
            [coord.x, coord.y, coord.z],
            radius,
            color,
            transparency,
        ));
    }

    spheres
}

/// Collect cylinder primitives from a molecule (sticks representation)
///
/// Extracts bonds visible as sticks representation.
pub fn collect_cylinders(
    molecule: &ObjectMolecule,
    coord_set: &CoordSet,
    colors: &ColorResolver<'_>,
    defaults: &RepColorDefaults,
    stick_radius: f32,
    stick_transparency: f32,
) -> (Vec<GpuCylinder>, Vec<GpuSphere>) {
    let mut cylinders = Vec::new();
    let mut atom_max_radius: HashMap<AtomIndex, (f32, [f32; 4], f32)> = HashMap::new();

    for bond in molecule.bonds() {
        let atom1_idx = bond.atom1;
        let atom2_idx = bond.atom2;

        let atom1 = match molecule.get_atom(atom1_idx) {
            Some(a) => a,
            None => continue,
        };
        let atom2 = match molecule.get_atom(atom2_idx) {
            Some(a) => a,
            None => continue,
        };

        // Check if sticks representation is visible
        if !atom1.repr.visible_reps.is_visible(RepMask::STICKS)
            && !atom2.repr.visible_reps.is_visible(RepMask::STICKS)
        {
            continue;
        }

        let pos1 = match coord_set.get_atom_coord(atom1_idx) {
            Some(p) => p,
            None => continue,
        };
        let pos2 = match coord_set.get_atom_coord(atom2_idx) {
            Some(p) => p,
            None => continue,
        };

        let color1 = colors.resolve_rep_color(atom1, atom1.repr.colors.stick, defaults.stick);
        let color2 = colors.resolve_rep_color(atom2, atom2.repr.colors.stick, defaults.stick);
        let radius = stick_radius * bond.order.as_float().sqrt();

        // Use per-atom transparency if set, otherwise fall back to global
        let trans1 = atom1.repr.stick_transparency.unwrap_or(stick_transparency);
        let trans2 = atom2.repr.stick_transparency.unwrap_or(stick_transparency);
        let bond_transparency = trans1.max(trans2);

        cylinders.push(GpuCylinder::new(
            [pos1.x, pos1.y, pos1.z],
            [pos2.x, pos2.y, pos2.z],
            radius,
            color1,
            color2,
            bond_transparency,
        ));

        // Track atoms for sphere caps (with per-atom transparency)
        atom_max_radius
            .entry(atom1_idx)
            .and_modify(|(r, _, _)| *r = r.max(radius))
            .or_insert((radius, color1, trans1));
        atom_max_radius
            .entry(atom2_idx)
            .and_modify(|(r, _, _)| *r = r.max(radius))
            .or_insert((radius, color2, trans2));
    }

    // Create sphere caps
    let mut sphere_caps = Vec::new();
    for (atom_idx, (max_radius, color, transparency)) in atom_max_radius {
        if let Some(coord) = coord_set.get_atom_coord(atom_idx) {
            sphere_caps.push(GpuSphere::new(
                [coord.x, coord.y, coord.z],
                max_radius,
                color,
                transparency,
            ));
        }
    }

    (cylinders, sphere_caps)
}

/// Options for primitive collection
#[derive(Clone, Debug)]
pub struct CollectOptions {
    /// Sphere scale factor
    pub sphere_scale: f32,
    /// Stick radius
    pub stick_radius: f32,
    /// Default sphere transparency
    pub sphere_transparency: f32,
    /// Default stick transparency
    pub stick_transparency: f32,
    /// Collect spheres representation
    pub collect_spheres: bool,
    /// Collect sticks representation  
    pub collect_sticks: bool,
    /// Collect cartoon representation
    pub collect_cartoon: bool,
    /// Collect surface representation
    pub collect_surface: bool,
}

impl Default for CollectOptions {
    fn default() -> Self {
        Self {
            sphere_scale: 1.0,
            stick_radius: 0.25,
            sphere_transparency: 0.0,
            stick_transparency: 0.0,
            collect_spheres: true,
            collect_sticks: true,
            collect_cartoon: true,
            collect_surface: true,
        }
    }
}

/// Collect all primitives from a molecule
pub fn collect_from_molecule(
    molecule: &ObjectMolecule,
    coord_set: &CoordSet,
    colors: &ColorResolver<'_>,
    defaults: &RepColorDefaults,
    options: &CollectOptions,
) -> Primitives {
    let mut collector = PrimitiveCollector::new();

    // Collect spheres
    if options.collect_spheres {
        let spheres = collect_spheres(
            molecule,
            coord_set,
            colors,
            defaults,
            options.sphere_scale,
            options.sphere_transparency,
        );
        collector.add_spheres(spheres);
    }

    // Collect sticks (cylinders + sphere caps)
    if options.collect_sticks {
        let (cylinders, caps) = collect_cylinders(
            molecule,
            coord_set,
            colors,
            defaults,
            options.stick_radius,
            options.stick_transparency,
        );
        collector.add_cylinders(cylinders);
        collector.add_spheres(caps);
    }

    // TODO: Collect cartoon representation (triangles)
    // TODO: Collect surface representation (triangles)

    collector.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_options_default() {
        let opts = CollectOptions::default();
        assert_eq!(opts.sphere_scale, 1.0);
        assert_eq!(opts.stick_radius, 0.25);
    }
}
