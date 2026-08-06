//! Map visualization commands: isomesh, isodot

use patinae_scene::MapDisplayMode;

use crate::args::ParsedCommand;
use crate::command::{ArgHint, Command, CommandContext, CommandRegistry, ViewerLike};
use crate::command_help;
use crate::error::{CmdError, CmdResult};

/// Register map commands
pub fn register(registry: &mut CommandRegistry) {
    registry.register(IsomeshCommand);
    registry.register(IsodotCommand);
    registry.register(IsosurfaceCommand);
}

/// Shared implementation for isomesh/isodot/isosurface commands
fn execute_isocontour(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    args: &ParsedCommand,
    mode: MapDisplayMode,
) -> CmdResult {
    if mode == MapDisplayMode::Isodot {
        return Err(CmdError::execution(
            "isodot map extraction is not implemented yet",
        ));
    }

    // isomesh name, map, level [, selection [, buffer [, carve ]]]
    let name = args
        .str_arg(0, "name")
        .ok_or_else(|| CmdError::missing_argument("name".to_string()))?;

    let map_name = args
        .str_arg(1, "map")
        .ok_or_else(|| CmdError::missing_argument("map".to_string()))?;

    let level = args.float_arg_or(2, "level", 1.0) as f32;

    if args.str_arg(3, "selection").is_some()
        || args.arg(4, "buffer").is_some()
        || args.arg(5, "carve").is_some()
    {
        return Err(CmdError::execution(
            "selection, buffer, and carve are not implemented for map extraction yet",
        ));
    }

    // Get the source map's grid data
    let map_data = {
        let map_obj =
            ctx.viewer.objects().get_map(map_name).ok_or_else(|| {
                CmdError::execution(format!("'{}' is not a map object", map_name))
            })?;

        map_obj
            .current_data()
            .ok_or_else(|| CmdError::execution(format!("Map '{}' has no data", map_name)))?
            .clone()
    };

    // Create a new MapObject for the visualization
    let mut vis_obj = patinae_scene::MapObject::from_map_data(name, map_data);
    vis_obj.set_level(level);
    vis_obj.set_display_mode(mode);

    ctx.viewer.insert_object(Box::new(vis_obj));

    let mode_name = match mode {
        MapDisplayMode::None => "map",
        MapDisplayMode::Isomesh => "isomesh",
        MapDisplayMode::Isodot => "isodot",
        MapDisplayMode::Isosurface => "isosurface",
        MapDisplayMode::Volume => "volume",
    };

    ctx.print(&format!(
        " Created {} \"{}\" from map \"{}\" at level {:.2}",
        mode_name, name, map_name, level
    ));

    Ok(())
}

// ============================================================================
// isomesh command
// ============================================================================

struct IsomeshCommand;

impl Command for IsomeshCommand {
    fn name(&self) -> &str {
        "isomesh"
    }

    command_help! {
        CMD "isomesh"
        DESCRIPTION [
            "creates a mesh representation of an electron density map",
            "at a specified contour level.",
        ]
        USAGE [
            "isomesh name, map, level [, selection [, buffer [, carve ]]]",
        ]
        REQUIRED [
            { "name", "string", "name for the new mesh object" },
            { "map", "string", "name of the map object" },
        ]
        OPTIONAL [
            { "level", "float", "contour level in sigma", "1.0" },
            { "selection", "string", "atoms around which to show density", "" },
            { "buffer", "float", "buffer distance around selection", "0" },
            { "carve", "float", "carve radius - hide density beyond this distance", "0" },
        ]
        EXAMPLES [
            "load density.ccp4",
            "isomesh mesh1, density, 1.0",
            "isomesh mesh2, density, 1.5",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Object, ArgHint::Object]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        execute_isocontour(ctx, args, MapDisplayMode::Isomesh)
    }
}

// ============================================================================
// isodot command
// ============================================================================

struct IsodotCommand;

impl Command for IsodotCommand {
    fn name(&self) -> &str {
        "isodot"
    }

    command_help! {
        CMD "isodot"
        DESCRIPTION [
            "creates a dot representation of an electron density map",
            "at a specified contour level (not implemented yet).",
        ]
        USAGE [
            "isodot name, map, level [, selection [, buffer [, carve ]]]",
        ]
        REQUIRED [
            { "name", "string", "name for the new dot object" },
            { "map", "string", "name of the map object" },
        ]
        OPTIONAL [
            { "level", "float", "contour level in sigma", "1.0" },
            { "selection", "string", "atoms around which to show density", "" },
            { "buffer", "float", "buffer distance around selection", "0" },
            { "carve", "float", "carve radius", "0" },
        ]
        EXAMPLES [
            "load density.ccp4",
            "isodot dots1, density, 1.0",
            "isodot dots2, density, 2.0, chain A, carve=2.0",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Object, ArgHint::Object]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        execute_isocontour(ctx, args, MapDisplayMode::Isodot)
    }
}

// ============================================================================
// isosurface command
// ============================================================================

struct IsosurfaceCommand;

impl Command for IsosurfaceCommand {
    fn name(&self) -> &str {
        "isosurface"
    }

    command_help! {
        CMD "isosurface"
        DESCRIPTION [
            "creates a solid surface representation of an electron",
            "density map at a specified contour level.",
        ]
        USAGE [
            "isosurface name, map, level [, selection [, buffer [, carve ]]]",
        ]
        REQUIRED [
            { "name", "string", "name for the new surface object" },
            { "map", "string", "name of the map object" },
        ]
        OPTIONAL [
            { "level", "float", "contour level in sigma", "1.0" },
            { "selection", "string", "atoms around which to show density", "" },
            { "buffer", "float", "buffer distance around selection", "0" },
            { "carve", "float", "carve radius", "0" },
        ]
        EXAMPLES [
            "load density.ccp4",
            "isosurface surf1, density, 1.0",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Object, ArgHint::Object]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        execute_isocontour(ctx, args, MapDisplayMode::Isosurface)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use lin_alg::f32::Vec3;
    use patinae_algos::surface::{extract_isomesh, extract_isosurface, Grid3D};
    use patinae_mol::{AtomBuilder, AtomIndex, CoordSet, Element, ObjectMolecule};
    use patinae_scene::{
        canonical_atom_path_for_hit, MapDisplayMode, MapObject, MoleculeObject, ObjectType,
        PickHit, Session, SessionAdapter,
    };
    use patinae_settings::groups::RecentPickLimit;

    use crate::CommandExecutor;

    fn seed_session_with_map() -> Session {
        let mut session = Session::new();
        let grid = Grid3D::from_dims([0.0; 3], [1.0; 3], [1, 1, 1], vec![0.0; 8]);
        session.registry.add(MapObject::new("src", grid));
        session
    }

    fn seed_picked_molecule(session: &mut Session, name: &str) {
        let mut molecule = ObjectMolecule::new(name);
        molecule.add_atom(
            AtomBuilder::new()
                .name("CA")
                .element(Element::Carbon)
                .resn("GLY")
                .resv(1)
                .chain("A")
                .build(),
        );
        molecule.add_coord_set(CoordSet::from_vec3(&[Vec3::new(0.0, 0.0, 0.0)]));
        session
            .registry
            .add(MoleculeObject::with_name(molecule, name));
        let molecule = session.registry.get_molecule(name).unwrap();
        let path = canonical_atom_path_for_hit(
            &PickHit {
                object_name: name.to_string(),
                object_type: ObjectType::Molecule,
                atom_index: Some(AtomIndex(0)),
                position: Vec3::new(0.0, 0.0, 0.0),
                distance: 0.0,
            },
            molecule.molecule(),
        )
        .unwrap();
        session
            .recent_atoms
            .insert(path, RecentPickLimit::Unlimited);
    }

    fn with_adapter<T>(session: &mut Session, f: impl FnOnce(&mut SessionAdapter<'_>) -> T) -> T {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (800, 600),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };
        f(&mut adapter)
    }

    fn write_i32(header: &mut [u8], offset: usize, value: i32) {
        header[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_f32(header: &mut [u8], offset: usize, value: f32) {
        header[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn minimal_ccp4_fixture() -> Vec<u8> {
        const GRID_POINTS: i32 = 3;

        let mut header = vec![0_u8; 1024];
        for offset in [0, 4, 8, 28, 32, 36] {
            write_i32(&mut header, offset, GRID_POINTS);
        }
        write_i32(&mut header, 12, 2);
        for (offset, length) in [(40, 3.0), (44, 3.0), (48, 3.0)] {
            write_f32(&mut header, offset, length);
        }
        for offset in [52, 56, 60] {
            write_f32(&mut header, offset, 90.0);
        }
        for (offset, axis) in [(64, 1), (68, 2), (72, 3)] {
            write_i32(&mut header, offset, axis);
        }
        write_f32(&mut header, 76, 0.0);
        write_f32(&mut header, 80, 2.0);
        write_f32(&mut header, 84, 2.0 / 27.0);
        write_i32(&mut header, 88, 1);
        header[208..212].copy_from_slice(b"MAP ");
        header[212] = 0x44;
        header[213] = 0x41;
        write_f32(&mut header, 216, 0.38);

        let mut fixture = header;
        for index in 0..27 {
            let value = if index == 13 { 2.0_f32 } else { 0.0_f32 };
            fixture.extend_from_slice(&value.to_le_bytes());
        }
        fixture
    }

    #[test]
    fn isomesh_creates_renderable_map_with_shared_grid() {
        let mut session = seed_session_with_map();
        let src_grid = session
            .registry
            .get_map("src")
            .and_then(MapObject::grid_shared)
            .unwrap();

        with_adapter(&mut session, |adapter| {
            CommandExecutor::new()
                .do_with_options(adapter, "isomesh mesh1, src, 1.5", true)
                .unwrap();
        });

        let mesh = session.registry.get_map("mesh1").unwrap();
        assert_eq!(mesh.display_mode(), MapDisplayMode::Isomesh);
        assert!(mesh.is_renderable());
        assert_eq!(mesh.level(), 1.5);
        assert!(Arc::ptr_eq(&src_grid, &mesh.grid_shared().unwrap()));
    }

    #[test]
    fn isosurface_creates_renderable_map() {
        let mut session = seed_session_with_map();

        with_adapter(&mut session, |adapter| {
            CommandExecutor::new()
                .do_with_options(adapter, "isosurface surf1, src, 2.0", true)
                .unwrap();
        });

        let surface = session.registry.get_map("surf1").unwrap();
        assert_eq!(surface.display_mode(), MapDisplayMode::Isosurface);
        assert!(surface.is_renderable());
        assert_eq!(surface.level(), 2.0);
    }

    #[test]
    fn isomesh_same_name_replacement_prunes_recent_atom() {
        let mut session = seed_session_with_map();
        seed_picked_molecule(&mut session, "mesh1");

        with_adapter(&mut session, |adapter| {
            CommandExecutor::new()
                .do_with_options(adapter, "isomesh mesh1, src, 1.0", true)
                .unwrap();
        });

        assert!(session.registry.get_map("mesh1").is_some());
        assert!(session.recent_atoms.is_empty());
    }

    #[test]
    fn unsupported_map_extraction_args_fail_explicitly() {
        let mut session = seed_session_with_map();

        let err = with_adapter(&mut session, |adapter| {
            CommandExecutor::new().do_with_options(adapter, "isodot dots1, src, 1.0", true)
        })
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("isodot map extraction is not implemented yet"),
            "{err}"
        );

        let err = with_adapter(&mut session, |adapter| {
            CommandExecutor::new().do_with_options(
                adapter,
                "isomesh mesh2, src, 1.0, chain A, 2.0, 1.0",
                true,
            )
        })
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("selection, buffer, and carve are not implemented"),
            "{err}"
        );
        assert!(session.registry.get_map("mesh2").is_none());
    }

    #[test]
    fn smoke_load_ccp4_and_create_isomesh_and_isosurface() {
        let mut session = Session::new();
        let fixture = tempfile::Builder::new()
            .suffix(".ccp4")
            .tempfile()
            .expect("temporary CCP4 fixture should be created");
        std::fs::write(fixture.path(), minimal_ccp4_fixture())
            .expect("temporary CCP4 fixture should be written");
        let load_cmd = format!("load {}, density", fixture.path().display());

        with_adapter(&mut session, |adapter| {
            let mut executor = CommandExecutor::new();
            executor.do_with_options(adapter, &load_cmd, true).unwrap();
            executor
                .do_with_options(adapter, "isomesh mesh1, density, 1.0", true)
                .unwrap();
            executor
                .do_with_options(adapter, "isosurface surf1, density, 1.0", true)
                .unwrap();
        });

        let source = session.registry.get_map("density").unwrap();
        assert_eq!(source.display_mode(), MapDisplayMode::None);
        assert!(!source.is_renderable());

        let mesh = session.registry.get_map("mesh1").unwrap();
        assert_eq!(mesh.display_mode(), MapDisplayMode::Isomesh);
        assert!(mesh.is_renderable());
        assert!(!extract_isomesh(mesh.grid().unwrap(), mesh.level()).is_empty());

        let surface = session.registry.get_map("surf1").unwrap();
        assert_eq!(surface.display_mode(), MapDisplayMode::Isosurface);
        assert!(surface.is_renderable());
        assert!(!extract_isosurface(surface.grid().unwrap(), surface.level()).is_empty());
    }
}
