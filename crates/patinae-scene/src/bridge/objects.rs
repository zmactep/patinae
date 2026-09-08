//! Render-input emitters.
//!
//! These helpers walk the registry in render order and emit stable
//! `ObjectId`s. `ObjectId(0)` is the picking sentinel.

use patinae_mol::{CoordSet, ObjectMolecule};
use patinae_render::{
    ObjectId, RenderAtomColors, RenderMapInput, RenderMapMode, RenderObjectInput, RepColorLutEntry,
    SceneLod,
};
use patinae_settings::{ResolvedSettings, Settings};

use crate::object::{MapDisplayMode, MoleculeObject, Object, ObjectRegistry};

use super::{ResolvedSceneColors, ResolvedSceneMarkers};

type RenderableMoleculeData<'a> = (
    &'a MoleculeObject,
    &'a ObjectMolecule,
    &'a CoordSet,
    &'a [[f32; 4]],
    &'a [RepColorLutEntry],
);

/// Walk the registry in render order, emitting one `RenderObjectInput`
/// per enabled molecule object that has a displayed coord set and pre-
/// resolved colors. The closure also receives the object's registry name
/// — callers building an `ObjectId → name` lookup can record it without a
/// second walk.
pub fn visit_render_objects<'a>(
    registry: &'a ObjectRegistry,
    settings: &Settings,
    colors: &'a ResolvedSceneColors,
    markers: &'a ResolvedSceneMarkers,
    visit: &mut dyn FnMut(&'a str, RenderObjectInput<'a>),
) {
    let make_colors = |name: &'a str, _mol: &'a MoleculeObject| {
        Some(RenderAtomColors::Separate {
            base: colors.get(name)?,
            reps: colors.get_rep(name)?,
        })
    };
    let lod = scene_lod(registry, colors);

    for name in registry.names() {
        if let Some(input) =
            render_molecule_input(registry, settings, &make_colors, markers, name, lod)
        {
            visit(name, input);
        }
    }
}

/// Walk the registry in render order, emitting molecules and renderable maps.
pub fn visit_render_scene<'a>(
    registry: &'a ObjectRegistry,
    settings: &Settings,
    colors: &'a ResolvedSceneColors,
    markers: &'a ResolvedSceneMarkers,
    visit_object: &mut dyn FnMut(&'a str, RenderObjectInput<'a>),
    visit_map: &mut dyn FnMut(&'a str, RenderMapInput<'a>),
) {
    let make_colors = |name: &'a str, _mol: &'a MoleculeObject| {
        Some(RenderAtomColors::Separate {
            base: colors.get(name)?,
            reps: colors.get_rep(name)?,
        })
    };
    visit_render_scene_using(
        registry,
        settings,
        &make_colors,
        markers,
        scene_lod(registry, colors),
        visit_object,
        visit_map,
    );
}

/// Visits the scene with colors resolved directly into renderer staging storage.
pub fn visit_render_scene_deferred<'a>(
    registry: &'a ObjectRegistry,
    settings: &Settings,
    named: &'a patinae_color::NamedPalette,
    themed: &'a patinae_color::ThemedPalette,
    markers: &'a ResolvedSceneMarkers,
    visit_object: &mut dyn FnMut(&'a str, RenderObjectInput<'a>),
    visit_map: &mut dyn FnMut(&'a str, RenderMapInput<'a>),
) {
    let make_colors = |_name: &'a str, mol: &'a MoleculeObject| {
        let settings = patinae_settings::ResolvedSettings::resolve(settings, mol.overrides());
        Some(RenderAtomColors::Source(Box::new(
            super::colors::ObjectColorSource::new(mol.molecule(), &settings, named, themed),
        )))
    };
    let total = registry
        .names()
        .filter_map(|name| registry.get_molecule(name))
        .filter(|mol| mol.state().enabled && mol.display_coord_set().is_some())
        .map(|mol| mol.molecule().atom_count())
        .sum();
    visit_render_scene_using(
        registry,
        settings,
        &make_colors,
        markers,
        SceneLod::from_atom_count(total),
        visit_object,
        visit_map,
    );
}

fn visit_render_scene_using<'a>(
    registry: &'a ObjectRegistry,
    settings: &Settings,
    make_colors: &dyn Fn(&'a str, &'a MoleculeObject) -> Option<RenderAtomColors<'a>>,
    markers: &'a ResolvedSceneMarkers,
    lod: SceneLod,
    visit_object: &mut dyn FnMut(&'a str, RenderObjectInput<'a>),
    visit_map: &mut dyn FnMut(&'a str, RenderMapInput<'a>),
) {
    for name in registry.names() {
        if let Some(input) =
            render_molecule_input(registry, settings, make_colors, markers, name, lod)
        {
            visit_object(name, input);
            continue;
        }

        let Some(map_obj) = registry.get_map(name) else {
            continue;
        };
        if !map_obj.state().enabled || !map_obj.is_renderable() {
            continue;
        }
        let Some(grid) = map_obj.grid() else {
            continue;
        };
        let Some(mode) = render_map_mode(map_obj.display_mode()) else {
            continue;
        };
        let Some(id) = render_object_id(registry, name) else {
            continue;
        };
        visit_map(
            name,
            RenderMapInput {
                object_id: id,
                grid,
                mode,
                level: map_obj.level(),
                color: map_obj.mesh_color(),
                transform: mat4_to_cols(&map_obj.state().transform),
                geometry_revision: map_obj.geometry_revision(),
                material_revision: map_obj.material_revision(),
                dirty: map_obj.is_dirty(),
            },
        );
    }
}

fn render_object_id(registry: &ObjectRegistry, name: &str) -> Option<ObjectId> {
    registry.render_id(name).map(|id| ObjectId(id.get()))
}

fn renderable_molecule_data<'a>(
    registry: &'a ObjectRegistry,
    colors: &'a ResolvedSceneColors,
    name: &str,
) -> Option<RenderableMoleculeData<'a>> {
    let mol_obj = registry.get_molecule(name)?;
    if !mol_obj.state().enabled {
        return None;
    }
    let mol = mol_obj.molecule();
    let coord = mol_obj.display_coord_set()?;
    let atom_colors = colors.get(name)?;
    let atom_rep_colors = colors.get_rep(name)?;
    if atom_colors.len() != mol.atom_count() || atom_rep_colors.len() != mol.atom_count() {
        return None;
    }
    Some((mol_obj, mol, coord, atom_colors, atom_rep_colors))
}

fn render_molecule_input<'a>(
    registry: &'a ObjectRegistry,
    settings: &Settings,
    make_colors: &dyn Fn(&'a str, &'a MoleculeObject) -> Option<RenderAtomColors<'a>>,
    markers: &'a ResolvedSceneMarkers,
    name: &'a str,
    lod: SceneLod,
) -> Option<RenderObjectInput<'a>> {
    let mol_obj = registry.get_molecule(name)?;
    if !mol_obj.state().enabled {
        return None;
    }
    let mol = mol_obj.molecule();
    let coord = mol_obj.display_coord_set()?;
    let colors = make_colors(name, mol_obj)?;
    if colors.len() != mol.atom_count() {
        return None;
    }
    let id = render_object_id(registry, name)?;
    let mut dirty = mol_obj.dirty_flags();
    if markers.is_dirty(name) {
        dirty |= markers.dirty_flags(name);
    }
    Some(RenderObjectInput {
        object_id: id,
        instances: mol_obj.state().instances.as_ref(),
        molecule: mol,
        coord_set: coord,
        transform: mat4_to_cols(&mol_obj.state().transform),
        visible_reps: mol_obj.visible_reps(),
        draw_reps: mol_obj.draw_reps(),
        object_settings: mol_obj
            .overrides()
            .map(|overrides| ResolvedSettings::resolve(settings, Some(overrides))),
        colors,
        atom_markers: markers.get(name).unwrap_or(&[]),
        recent_atom_markers: None,
        marker_updates: markers.updates(name).unwrap_or(&[]),
        has_markers: markers.has_markers(name),
        lod,
        dirty,
    })
}

fn scene_lod(registry: &ObjectRegistry, colors: &ResolvedSceneColors) -> SceneLod {
    let mut total_atoms: usize = 0;
    for name in registry.names() {
        if let Some((_mol_obj, mol, _coord, _atom_colors, _atom_rep_colors)) =
            renderable_molecule_data(registry, colors, name)
        {
            total_atoms += mol.atom_count();
        }
    }
    SceneLod::from_atom_count(total_atoms)
}

fn render_map_mode(mode: MapDisplayMode) -> Option<RenderMapMode> {
    match mode {
        MapDisplayMode::Isomesh => Some(RenderMapMode::Isomesh),
        MapDisplayMode::Isosurface => Some(RenderMapMode::Isosurface),
        MapDisplayMode::None | MapDisplayMode::Isodot | MapDisplayMode::Volume => None,
    }
}

fn mat4_to_cols(m: &lin_alg::f32::Mat4) -> [[f32; 4]; 4] {
    let d = m.data;
    [
        [d[0], d[1], d[2], d[3]],
        [d[4], d[5], d[6], d[7]],
        [d[8], d[9], d[10], d[11]],
        [d[12], d[13], d[14], d[15]],
    ]
}
