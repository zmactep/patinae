//! File I/O commands: load, save, png, fetch, cd, pwd, ls

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use patinae_algos::surface::Grid3D;
use patinae_io::FileFormat;
use patinae_mol::dss::{assign_secondary_structure, assigner_for};
use patinae_mol::ObjectMolecule;
use patinae_scene::{MapData, MapObject, MoleculeObject, Object, ObjectRegistry};

use crate::args::ParsedCommand;
use crate::command::{
    ArgHint, BuiltinCommandCapability, Command, CommandContext, CommandRegistry, ViewerLike,
};
#[cfg(any(feature = "fetch", feature = "fetch-async"))]
use crate::command::{AsyncCommandRequest, FetchFormatCode, FetchRequest};
use crate::command_help;
use crate::error::{CmdError, CmdResult};

use super::objects::extract_molecule;
use super::selecting::evaluate_selection;

/// Expand shell-style paths: ~ to home directory, $VAR to environment variables
pub fn expand_path(path: &str) -> PathBuf {
    #[cfg(feature = "native-io")]
    {
        match shellexpand::full(path) {
            Ok(expanded) => PathBuf::from(expanded.as_ref()),
            Err(_) => PathBuf::from(path),
        }
    }
    #[cfg(not(feature = "native-io"))]
    {
        PathBuf::from(path)
    }
}

/// Apply DSS (Define Secondary Structure) algorithm to a molecule
///
/// This recalculates secondary structure based on backbone geometry,
/// using the algorithm specified by the `dss_algorithm` setting.
/// It overwrites the secondary structure assignments from the PDB/CIF file.
fn apply_dss(mol: &mut ObjectMolecule, algorithm: patinae_settings::DssAlgorithm) {
    let assigner = assigner_for(algorithm);
    assign_secondary_structure(mol, 0, assigner.as_ref());
}

/// Finalize a fetched molecule on the main viewer thread.
///
/// Both synchronous command execution and GUI async-task completion use this
/// path so DSS, object insertion, zooming, and movie-state updates stay
/// behaviorally identical.
pub fn finalize_fetched_molecule(
    viewer: &mut dyn ViewerLike,
    name: &str,
    mut mol: ObjectMolecule,
    auto_dss: bool,
    dss_algorithm: patinae_settings::DssAlgorithm,
) {
    if auto_dss {
        apply_dss(&mut mol, dss_algorithm);
    }

    viewer.insert_object(Box::new(MoleculeObject::with_name(mol, name)));
    viewer.zoom_on(name, 0.0);
    viewer.update_movie_state_count();
}

/// Register I/O commands
pub fn register(registry: &mut CommandRegistry) {
    registry.register(LoadCommand);
    #[cfg(feature = "traj")]
    registry.register(LoadTrajCommand);
    registry.register(SaveCommand);
    registry.register(PngCommand);
    registry.register(CdCommand);
    registry.register(PwdCommand);
    registry.register(LsCommand);
    #[cfg(any(feature = "fetch", feature = "fetch-async"))]
    registry.register(FetchCommand);
}

pub(crate) const LOAD_CAPABILITY: BuiltinCommandCapability = BuiltinCommandCapability {
    command: "load",
    aliases: &[],
    suffixes: &[
        "pdb", "pdb.gz", "ent", "ent.gz", "sdf", "sdf.gz", "mol", "mol.gz", "sd", "sd.gz", "mol2",
        "mol2.gz", "ml2", "ml2.gz", "cif", "cif.gz", "mmcif", "mmcif.gz", "bcif", "bcif.gz", "xyz",
        "xyz.gz", "gro", "gro.gz", "ccp4", "ccp4.gz", "map", "map.gz", "mrc", "mrc.gz", "pse",
        "pze", "prs",
    ],
    available: true,
};

pub(crate) const LOAD_TRAJ_CAPABILITY: BuiltinCommandCapability = BuiltinCommandCapability {
    command: "load_traj",
    aliases: &[],
    suffixes: &["xtc", "xtc.gz", "trr", "trr.gz"],
    available: cfg!(feature = "traj"),
};

pub(crate) const SAVE_CAPABILITY: BuiltinCommandCapability = BuiltinCommandCapability {
    command: "save",
    aliases: &[],
    suffixes: &[
        "pdb", "ent", "sdf", "mol", "sd", "mol2", "ml2", "cif", "mmcif", "xyz", "gro", "prs", "pml",
    ],
    available: true,
};

fn normalized_path_suffix(path: &Path) -> String {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let Some(extension) = Path::new(&filename)
        .extension()
        .and_then(|ext| ext.to_str())
    else {
        return String::new();
    };
    if extension != "gz" {
        return extension.to_string();
    }

    let inner = filename.strip_suffix(".gz").unwrap_or(&filename);
    Path::new(inner)
        .extension()
        .and_then(|ext| ext.to_str())
        .map_or_else(|| "gz".to_string(), |ext| format!("{ext}.gz"))
}

// ============================================================================
// load command
// ============================================================================

/// Load a molecular structure file
struct LoadCommand;

impl Command for LoadCommand {
    fn name(&self) -> &str {
        LOAD_CAPABILITY.command
    }

    command_help! {
        CMD "load"
        DESCRIPTION [
            "reads several molecular file formats and creates a new",
            "object or adds states to an existing object.",
        ]
        REQUIRED [
            { "filename", "string", "file path or URL" },
        ]
        OPTIONAL [
            { "object", "string", "object name", "filename stem" },
            { "state", "integer", "state to load into (0 = append)", "0" },
            { "format", "string", "file format (auto-detected if omitted)", "auto" },
            { "quiet", "0/1", "suppress feedback", "0" },
        ]
        EXAMPLES [
            "load protein.pdb",
            "load ligand.sdf, object=lig",
            "load structure.cif, format=cif",
            "load session.pse         → import PyMOL session",
            "load session.prs         → load native session",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Path, ArgHint::Object]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        // Get filename (required)
        let filename = args
            .str_arg(0, "filename")
            .ok_or_else(|| CmdError::missing_argument("filename".to_string()))?;

        // Get object name (optional, defaults to filename stem)
        let object_name = args
            .str_arg(1, "object")
            .map(|s| s.to_string())
            .unwrap_or_else(|| default_load_object_name(Path::new(filename), "obj"));

        // Get state (optional, 0 = append)
        let state = args.int_arg_or(2, "state", 0);

        if state != 0 {
            ctx.print(" Warning: state= parameter not yet fully supported, loading all states.");
        }

        // Get format (optional, auto-detect)
        let format = args
            .str_arg(3, "format")
            .map(|s| match s.to_lowercase().as_str() {
                "pdb" => FileFormat::Pdb,
                "sdf" | "mol" => FileFormat::Sdf,
                "mol2" => FileFormat::Mol2,
                "xyz" => FileFormat::Xyz,
                "cif" | "mmcif" => FileFormat::Cif,
                "gro" => FileFormat::Gro,
                "xtc" => FileFormat::Xtc,
                "trr" => FileFormat::Trr,
                "ccp4" | "map" | "mrc" => FileFormat::Ccp4,
                _ => FileFormat::Unknown,
            });

        // Get quiet flag
        let quiet = args.bool_arg_or(4, "quiet", false);

        // Expand path and check for session file formats
        let path = expand_path(filename);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "pse" | "pze" => {
                use patinae_session::{load_pse, pse_to_session};
                let pse = load_pse(&path).map_err(|e| CmdError::file_format(e.to_string()))?;
                let session =
                    pse_to_session(&pse).map_err(|e| CmdError::file_format(e.to_string()))?;
                let n_objs = session.registry.len();
                ctx.viewer.replace_session(session);
                if !quiet {
                    ctx.print(&format!(
                        " Loaded PSE session \"{}\" ({} objects)",
                        filename, n_objs
                    ));
                }
                ctx.viewer.update_movie_state_count();
                record_loaded_file(ctx, &path);
                return Ok(());
            }
            "prs" => {
                use patinae_session::load_prs_document;
                let document =
                    load_prs_document(&path).map_err(|e| CmdError::file_format(e.to_string()))?;
                for warning in document.warning_messages() {
                    ctx.print_warning(&warning);
                }
                let session = document.session;
                ctx.viewer.replace_session(session);
                ctx.viewer.update_movie_state_count();
                if !quiet {
                    ctx.print(&format!(" Loaded PRS session \"{}\"", filename));
                }
                record_loaded_file(ctx, &path);
                return Ok(());
            }
            _ => {}
        }

        // Detect format
        let builtin_format = format.unwrap_or_else(|| {
            let detected = FileFormat::from_path(&path);
            let suffix = normalized_path_suffix(&path);
            if LOAD_CAPABILITY.supports_suffix(&suffix) || detected.is_trajectory_only() {
                detected
            } else {
                FileFormat::Unknown
            }
        });

        // Reject trajectory-only formats (no topology)
        if builtin_format.is_trajectory_only() {
            return Err(CmdError::execution(format!(
                "{} is a trajectory-only format with no topology. Use 'load_traj' instead.",
                builtin_format.name()
            )));
        }

        // Handle CCP4/MRC map files as MapObject (not MoleculeObject)
        if builtin_format.is_map_format() {
            let ccp4 = patinae_io::ccp4::read_ccp4(&path)
                .map_err(|e| CmdError::file_format(e.to_string()))?;

            if !quiet {
                ctx.print(&format!(
                    " CCP4 map: {}x{}x{} grid, spacing ({:.2}, {:.2}, {:.2}) Å",
                    ccp4.dims[0] + 1,
                    ccp4.dims[1] + 1,
                    ccp4.dims[2] + 1,
                    ccp4.spacing[0],
                    ccp4.spacing[1],
                    ccp4.spacing[2]
                ));
                ctx.print(&format!(
                    " Density: mean {:.4}, rms {:.4} (normalized to sigma)",
                    ccp4.stats.dmean, ccp4.stats.rms
                ));
                ctx.print(&format!(
                    " Origin: ({:.2}, {:.2}, {:.2})",
                    ccp4.origin[0], ccp4.origin[1], ccp4.origin[2]
                ));
            }

            let grid = Grid3D::from_dims(ccp4.origin, ccp4.spacing, ccp4.dims, ccp4.values);
            let map_data = MapData::new(grid);
            let map_obj = MapObject::from_map_data(&object_name, map_data);
            ctx.viewer.insert_object(Box::new(map_obj));

            if !quiet {
                ctx.print(&format!(
                    " Loaded map \"{}\" as \"{}\"",
                    filename, object_name
                ));
            }

            ctx.viewer.zoom_on(&object_name, 0.0);
            record_loaded_file(ctx, &path);
            return Ok(());
        }

        // Load molecular file
        let mut molecules = if builtin_format != FileFormat::Unknown {
            // Built-in format
            patinae_io::read_all_format_with_bond_tolerance(
                &path,
                builtin_format,
                ctx.viewer.settings().behavior.bonding_vdw_cutoff,
            )
            .map_err(|e| CmdError::file_format(e.to_string()))?
        } else if let Some(handler) = ctx.format_handler(&ext) {
            // Plugin-provided format
            let reader_fn = handler.reader.as_ref().ok_or_else(|| {
                CmdError::execution(format!(
                    "Format '{}' does not support reading",
                    handler.name
                ))
            })?;
            let file =
                std::fs::File::open(&path).map_err(|e| CmdError::file_format(e.to_string()))?;
            let buf_reader: Box<dyn std::io::Read> = Box::new(std::io::BufReader::new(file));
            (reader_fn)(buf_reader).map_err(CmdError::file_format)?
        } else {
            return Err(CmdError::execution(format!(
                "Unknown file format: .{} (no built-in or plugin handler found)",
                ext
            )));
        };

        if molecules.is_empty() {
            return Err(CmdError::file_format("No molecules in file".to_string()));
        }

        // Apply DSS (Define Secondary Structure) if auto_dss is enabled
        // This recalculates secondary structure based on backbone geometry
        let behavior = &ctx.viewer.settings().behavior;
        if behavior.auto_dss {
            for mol in &mut molecules {
                apply_dss(mol, behavior.dss_algorithm);
            }
        }

        let mut inserted_names = Vec::with_capacity(molecules.len());
        let mut reserved_names = HashSet::with_capacity(molecules.len());
        let mut inserted_objects: Vec<Box<dyn Object>> = Vec::with_capacity(molecules.len());
        for (index, mol) in molecules.into_iter().enumerate() {
            let name = if index == 0 {
                object_name.clone()
            } else {
                derive_extra_object_name(
                    &object_name,
                    index + 1,
                    ctx.viewer.objects(),
                    &reserved_names,
                )
            };
            let mol_obj = MoleculeObject::with_name(mol, &name);
            inserted_objects.push(Box::new(mol_obj));
            reserved_names.insert(name.clone());
            inserted_names.push(name);
        }
        ctx.viewer.insert_objects(inserted_objects);

        if !quiet {
            print_loaded_molecules_message(ctx, filename, &inserted_names);
        }

        // Zoom to loaded molecule (preserves rotation)
        ctx.viewer.zoom_on(&inserted_names[0], 0.0);
        ctx.viewer.update_movie_state_count();
        record_loaded_file(ctx, &path);

        Ok(())
    }
}

fn default_load_object_name(path: &Path, fallback: &str) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(fallback);
    let name = if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gz"))
    {
        Path::new(stem)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(stem)
    } else {
        stem
    };
    sanitize_load_object_name(name)
}

fn sanitize_load_object_name(name: &str) -> String {
    name.replace('-', "_")
}

fn derive_extra_object_name(
    base: &str,
    index: usize,
    registry: &ObjectRegistry,
    reserved: &HashSet<String>,
) -> String {
    let mut suffix = index;
    loop {
        let candidate = format!("{base}_{suffix}");
        if !registry.contains(&candidate) && !reserved.contains(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn print_loaded_molecules_message(
    ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>,
    filename: &str,
    object_names: &[String],
) {
    if object_names.len() == 1 {
        ctx.print(&format!(
            " Loaded \"{}\" as \"{}\"",
            filename, object_names[0]
        ));
    } else {
        ctx.print(&format!(
            " Loaded \"{}\" as {} objects: {}",
            filename,
            object_names.len(),
            object_names.join(", ")
        ));
    }
}

fn record_loaded_file(ctx: &mut CommandContext<'_, '_, dyn ViewerLike + '_>, path: &Path) {
    let path = recent_file_path(path);
    ctx.record_recent_file(path.to_string_lossy().into_owned(), "load");
}

fn recent_file_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    if path.is_absolute() {
        return path.to_path_buf();
    }
    env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod load_tests {
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use flate2::write::GzEncoder;
    use flate2::Compression;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, AtomIndex, CoordSet, Element, ObjectMolecule};
    use patinae_scene::{MoleculeObject, Session, SessionAdapter};

    use crate::{CommandAction, CommandExecutor, CommandOutput, FormatHandler, MessageKind};

    use super::*;

    #[test]
    fn builtin_io_capabilities_match_supported_command_suffixes() {
        assert_eq!(LOAD_CAPABILITY.command, "load");
        assert!(LOAD_CAPABILITY.suffixes.contains(&"pdb"));
        assert!(LOAD_CAPABILITY.suffixes.contains(&"pdb.gz"));
        for alias in ["ent", "sd", "ml2", "mmcif"] {
            assert!(LOAD_CAPABILITY.suffixes.contains(&alias));
            assert!(SAVE_CAPABILITY.suffixes.contains(&alias));
        }
        assert!(LOAD_CAPABILITY.suffixes.contains(&"bcif.gz"));
        assert!(LOAD_CAPABILITY.suffixes.contains(&"mrc.gz"));
        assert!(LOAD_CAPABILITY.suffixes.contains(&"pze"));
        for unsupported in ["xtc", "trr", "png"] {
            assert!(!LOAD_CAPABILITY.suffixes.contains(&unsupported));
        }

        assert_eq!(SAVE_CAPABILITY.command, "save");
        assert!(SAVE_CAPABILITY.suffixes.contains(&"pml"));
        assert!(SAVE_CAPABILITY.suffixes.contains(&"prs"));
        for unsupported in [
            "pse", "pze", "bcif", "xtc", "trr", "ccp4", "map", "mrc", "pdb.gz", "png",
        ] {
            assert!(!SAVE_CAPABILITY.suffixes.contains(&unsupported));
        }

        assert_eq!(LOAD_TRAJ_CAPABILITY.command, "load_traj");
        assert_eq!(LOAD_TRAJ_CAPABILITY.available, cfg!(feature = "traj"));
        assert_eq!(
            CommandRegistry::with_builtins().contains("load_traj"),
            LOAD_TRAJ_CAPABILITY.available
        );
        assert_eq!(
            LOAD_TRAJ_CAPABILITY.suffixes,
            &["xtc", "xtc.gz", "trr", "trr.gz"]
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "patinae_cmd_load_{name}_{}_{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temporary directory should be created");
        dir
    }

    fn single_atom_molecule(name: &str, x: f32) -> ObjectMolecule {
        let mut mol = ObjectMolecule::new(name);
        mol.add_atom(Atom::new("C", Element::Carbon));
        mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(x, 0.0, 0.0)]));
        mol
    }

    fn write_multi_molecule_fixture(path: &Path, format: FileFormat) {
        let molecules = [
            single_atom_molecule("first", 0.0),
            single_atom_molecule("second", 1.0),
            single_atom_molecule("third", 2.0),
        ];
        patinae_io::write_all_format(path, &molecules, format)
            .expect("multi-molecule fixture should be writable");
    }

    fn write_gzipped_cif(path: &Path) {
        let cif = r#"data_ONE
loop_
_atom_site.id
_atom_site.type_symbol
_atom_site.label_atom_id
_atom_site.label_comp_id
_atom_site.label_asym_id
_atom_site.label_seq_id
_atom_site.Cartn_x
_atom_site.Cartn_y
_atom_site.Cartn_z
1 C C LIG A 1 0.0 0.0 0.0
"#;
        let file = fs::File::create(path).expect("gzipped CIF fixture should be created");
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder
            .write_all(cif.as_bytes())
            .expect("CIF payload should be written");
        encoder.finish().expect("gzip stream should finish");
    }

    fn run_command(
        session: &mut Session,
        executor: &mut CommandExecutor,
        command: &str,
    ) -> CommandOutput {
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };

        executor
            .do_with_options(&mut adapter, command, false)
            .expect("load command should succeed")
    }

    fn object_names(session: &Session) -> Vec<String> {
        session
            .registry
            .names()
            .map(str::to_string)
            .collect::<Vec<_>>()
    }

    fn assert_recent_file_recorded_once(output: &CommandOutput) {
        let recent_file_actions = output
            .actions
            .iter()
            .filter(|action| matches!(action, CommandAction::RecordRecentFile { .. }))
            .count();
        assert_eq!(recent_file_actions, 1);
    }

    #[test]
    fn load_multi_sdf_creates_derived_objects_and_summary() {
        let dir = temp_dir("multi_sdf");
        let path = dir.join("ligands.sdf");
        write_multi_molecule_fixture(&path, FileFormat::Sdf);

        let mut session = Session::new();
        let mut executor = CommandExecutor::new();
        let command = format!("load {}", path.display());
        let output = run_command(&mut session, &mut executor, &command);

        assert_eq!(
            object_names(&session),
            ["ligands", "ligands_2", "ligands_3"]
        );
        assert_eq!(output.messages.len(), 1);
        assert_eq!(output.messages[0].kind, MessageKind::Info);
        assert_eq!(
            output.messages[0].text,
            format!(
                " Loaded \"{}\" as 3 objects: ligands, ligands_2, ligands_3",
                path.display()
            )
        );
        assert_recent_file_recorded_once(&output);

        fs::remove_dir_all(&dir).expect("temporary directory should be removed");
    }

    #[test]
    fn load_multi_mol2_creates_multiple_objects() {
        let dir = temp_dir("multi_mol2");
        let path = dir.join("ligands.mol2");
        write_multi_molecule_fixture(&path, FileFormat::Mol2);

        let mut session = Session::new();
        let mut executor = CommandExecutor::new();
        let command = format!("load {}", path.display());
        let output = run_command(&mut session, &mut executor, &command);

        assert_eq!(
            object_names(&session),
            ["ligands", "ligands_2", "ligands_3"]
        );
        assert_recent_file_recorded_once(&output);

        fs::remove_dir_all(&dir).expect("temporary directory should be removed");
    }

    #[test]
    fn load_multi_sdf_uses_explicit_name_for_first_object() {
        let dir = temp_dir("explicit_name");
        let path = dir.join("ligands.sdf");
        write_multi_molecule_fixture(&path, FileFormat::Sdf);

        let mut session = Session::new();
        let mut executor = CommandExecutor::new();
        let command = format!("load {}, object=foo", path.display());
        let output = run_command(&mut session, &mut executor, &command);

        assert_eq!(object_names(&session), ["foo", "foo_2", "foo_3"]);
        assert_eq!(
            output.messages[0].text,
            format!(
                " Loaded \"{}\" as 3 objects: foo, foo_2, foo_3",
                path.display()
            )
        );

        fs::remove_dir_all(&dir).expect("temporary directory should be removed");
    }

    #[test]
    fn load_replaces_first_object_and_keeps_existing_extra_name() {
        let dir = temp_dir("replacement");
        let path = dir.join("ligands.sdf");
        let molecules = [
            single_atom_molecule("new_first", 1.0),
            single_atom_molecule("new_second", 2.0),
        ];
        patinae_io::write_all_format(&path, &molecules, FileFormat::Sdf)
            .expect("SDF fixture should be writable");

        let mut session = Session::new();
        session.registry.add(MoleculeObject::with_name(
            single_atom_molecule("old_first", 8.0),
            "foo",
        ));
        session.registry.add(MoleculeObject::with_name(
            single_atom_molecule("old_second", 9.0),
            "foo_2",
        ));

        let mut executor = CommandExecutor::new();
        let command = format!("load {}, object=foo", path.display());
        let output = run_command(&mut session, &mut executor, &command);

        assert_eq!(object_names(&session), ["foo_2", "foo", "foo_3"]);
        let foo = session.registry.get_molecule("foo").unwrap().molecule();
        let foo_2 = session.registry.get_molecule("foo_2").unwrap().molecule();
        let foo_3 = session.registry.get_molecule("foo_3").unwrap().molecule();
        assert_eq!(foo.get_coord(AtomIndex::new(0), 0).unwrap().x, 1.0);
        assert_eq!(foo_2.get_coord(AtomIndex::new(0), 0).unwrap().x, 9.0);
        assert_eq!(foo_3.get_coord(AtomIndex::new(0), 0).unwrap().x, 2.0);
        assert_eq!(
            output.messages[0].text,
            format!(" Loaded \"{}\" as 2 objects: foo, foo_3", path.display())
        );

        fs::remove_dir_all(&dir).expect("temporary directory should be removed");
    }

    #[test]
    fn load_plugin_format_consumes_all_returned_molecules() {
        let dir = temp_dir("plugin");
        let path = dir.join("structure.foo");
        fs::write(&path, b"plugin fixture").expect("plugin fixture should be written");

        let mut session = Session::new();
        let mut executor = CommandExecutor::new();
        executor.register_format_handler(FormatHandler {
            name: "Foo".to_string(),
            extensions: vec!["foo".to_string()],
            reader: Some(Arc::new(|_| {
                Ok(vec![
                    single_atom_molecule("plugin_first", 0.0),
                    single_atom_molecule("plugin_second", 1.0),
                ])
            })),
            writer: None,
        });

        let command = format!("load {}", path.display());
        let output = run_command(&mut session, &mut executor, &command);

        assert_eq!(object_names(&session), ["structure", "structure_2"]);
        assert_eq!(
            output.messages[0].text,
            format!(
                " Loaded \"{}\" as 2 objects: structure, structure_2",
                path.display()
            )
        );

        fs::remove_dir_all(&dir).expect("temporary directory should be removed");
    }

    #[test]
    fn load_builtin_format_keeps_precedence_over_plugin_handler() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let dir = temp_dir("builtin_precedence");
        let path = dir.join("structure.pdb");
        patinae_io::write_all_format(
            &path,
            &[single_atom_molecule("builtin", 0.0)],
            FileFormat::Pdb,
        )
        .expect("PDB fixture should be writable");
        let plugin_called = Arc::new(AtomicBool::new(false));
        let plugin_called_by_reader = plugin_called.clone();
        let mut session = Session::new();
        let mut executor = CommandExecutor::new();
        executor.register_format_handler(FormatHandler {
            name: "Shadow PDB".to_string(),
            extensions: vec!["pdb".to_string()],
            reader: Some(Arc::new(move |_| {
                plugin_called_by_reader.store(true, Ordering::SeqCst);
                Ok(vec![single_atom_molecule("plugin", 1.0)])
            })),
            writer: None,
        });

        let command = format!("load {}", path.display());
        run_command(&mut session, &mut executor, &command);

        assert!(!plugin_called.load(Ordering::SeqCst));
        assert_eq!(object_names(&session), ["structure"]);

        fs::remove_dir_all(&dir).expect("temporary directory should be removed");
    }

    #[test]
    fn load_cif_gz_default_name_strips_double_extension_and_sanitizes_hyphens() {
        let dir = temp_dir("cif_gz");
        let path = dir.join("my-name.cif.gz");
        write_gzipped_cif(&path);

        let mut session = Session::new();
        let mut executor = CommandExecutor::new();
        let command = format!("load {}", path.display());
        let output = run_command(&mut session, &mut executor, &command);

        assert_eq!(object_names(&session), ["my_name"]);
        assert_eq!(
            output.messages[0].text,
            format!(" Loaded \"{}\" as \"my_name\"", path.display())
        );

        fs::remove_dir_all(&dir).expect("temporary directory should be removed");
    }

    #[test]
    fn load_quiet_argument_suppresses_info_messages() {
        let dir = temp_dir("quiet");
        let path = dir.join("ligands.sdf");
        write_multi_molecule_fixture(&path, FileFormat::Sdf);

        let mut session = Session::new();
        let mut executor = CommandExecutor::new();
        let command = format!("load {}, quiet=1", path.display());
        let output = run_command(&mut session, &mut executor, &command);

        assert_eq!(
            object_names(&session),
            ["ligands", "ligands_2", "ligands_3"]
        );
        assert!(output.messages.is_empty());
        assert_recent_file_recorded_once(&output);

        fs::remove_dir_all(&dir).expect("temporary directory should be removed");
    }
}

// ============================================================================
// load_traj command
// ============================================================================

#[cfg(feature = "traj")]
/// Load trajectory coordinates onto an existing molecular object
struct LoadTrajCommand;

#[cfg(feature = "traj")]
impl Command for LoadTrajCommand {
    fn name(&self) -> &str {
        LOAD_TRAJ_CAPABILITY.command
    }

    command_help! {
        CMD "load_traj"
        DESCRIPTION [
            "loads trajectory coordinates from a file onto an",
            "existing molecular object. The trajectory must have the same",
            "number of atoms as the target object.",
        ]
        REQUIRED [
            { "filename", "string", "trajectory file path (XTC, TRR)" },
        ]
        OPTIONAL [
            { "object", "string", "target object name", "first molecule" },
            { "start", "integer", "first frame to load, 1-based", "1" },
            { "stop", "integer", "last frame to load, 1-based", "all" },
            { "interval", "integer", "load every Nth frame", "1" },
            { "format", "string", "file format (auto-detected if omitted)", "auto" },
            { "append", "bool", "append loaded states to existing", "false" },
        ]
        EXAMPLES [
            "load protein.gro",
            "load_traj trajectory.xtc, protein",
            "load_traj long_sim.xtc, protein, start=1, stop=1000, interval=10",
            "load_traj trajectory.trr, protein",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Path, ArgHint::Object]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        // Get filename (required)
        let filename = args
            .str_arg(0, "filename")
            .ok_or_else(|| CmdError::missing_argument("filename".to_string()))?;

        // Get object name (optional, defaults to first loaded molecule)
        let object_name = args.str_arg(1, "object");

        // PyMOL uses 1-based frame numbers; convert to 0-based
        let start = args
            .int_arg(2, "start")
            .map(|s| (s.max(1) - 1) as usize)
            .unwrap_or(0);

        let stop = args.int_arg(3, "stop").map(|s| s.max(0) as usize);

        let interval = args
            .int_arg(4, "interval")
            .map(|i| i.max(1) as usize)
            .unwrap_or(1);

        let append = args
            .get_bool(5)
            .unwrap_or(args.get_named_bool_or("append", false));

        let quiet = args.get_named_bool("quiet").unwrap_or(false);

        // Resolve target object name
        let target_name = if let Some(name) = object_name {
            name.to_string()
        } else {
            // Find first molecule object
            let mut first_mol = None;
            for name in ctx.viewer.objects().names() {
                if ctx.viewer.objects().get_molecule(name).is_some() {
                    first_mol = Some(name.to_string());
                    break;
                }
            }
            first_mol.ok_or_else(|| {
                CmdError::execution("No molecule objects loaded. Load a structure first.")
            })?
        };

        // Validate target exists and get atom count
        let expected_atoms = {
            let mol_obj = ctx
                .viewer
                .objects()
                .get_molecule(&target_name)
                .ok_or_else(|| CmdError::object_not_found(target_name.clone()))?;
            mol_obj.molecule().atom_count()
        };

        // Detect format
        let path = expand_path(filename);
        let format = args
            .get_named_str("format")
            .map(|s| match s.to_lowercase().as_str() {
                "xtc" => FileFormat::Xtc,
                "trr" => FileFormat::Trr,
                _ => FileFormat::from_path(&path),
            })
            .unwrap_or_else(|| {
                let suffix = normalized_path_suffix(&path);
                if LOAD_TRAJ_CAPABILITY.supports_suffix(&suffix) {
                    FileFormat::from_path(&path)
                } else {
                    FileFormat::Unknown
                }
            });

        // Read trajectory frames
        let opts = patinae_io::TrajectoryReadOptions {
            start,
            stop,
            interval,
        };

        let frames = patinae_io::read_trajectory_format(&path, format, &opts)
            .map_err(|e| CmdError::file_format(e.to_string()))?;

        if frames.is_empty() {
            return Err(CmdError::execution("No frames found in trajectory"));
        }

        // Validate atom count
        let traj_atoms = frames[0].len();
        if traj_atoms != expected_atoms {
            return Err(CmdError::execution(format!(
                "Atom count mismatch: object '{}' has {} atoms, trajectory has {}",
                target_name, expected_atoms, traj_atoms
            )));
        }

        // Replace existing coordinates with trajectory frames
        let n_frames = frames.len();
        let mol_obj = ctx
            .viewer
            .objects_mut()
            .get_molecule_mut(&target_name)
            .ok_or_else(|| CmdError::object_not_found(target_name.clone()))?;

        if !append {
            mol_obj.molecule_mut().clear_coord_sets();
        }
        for cs in frames {
            mol_obj.molecule_mut().add_coord_set(cs);
        }

        let total_states = mol_obj.molecule().state_count();

        if !quiet {
            ctx.print(&format!(
                " Loaded {} frames from \"{}\" onto \"{}\" ({} total states)",
                n_frames, filename, target_name, total_states
            ));
        }

        ctx.viewer.update_movie_state_count();

        Ok(())
    }
}

// ============================================================================
// save command
// ============================================================================

/// Save a molecular structure to file
struct SaveCommand;

impl Command for SaveCommand {
    fn name(&self) -> &str {
        SAVE_CAPABILITY.command
    }

    command_help! {
        CMD "save"
        DESCRIPTION [
            "writes molecular data to a file.",
        ]
        REQUIRED [
            { "filename", "string", "output file path" },
        ]
        OPTIONAL [
            { "selection", "string", "atoms to save", "all" },
            { "state", "integer", "state to save", "-1 = current" },
            { "format", "string", "file format (auto-detected from extension)", "auto" },
        ]
        EXAMPLES [
            "save output.pdb",
            "save ligand.sdf, organic",
            "save protein.mol2, polymer",
            "save session.prs         → save native session",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Path, ArgHint::Selection]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let filename = args
            .str_arg(0, "filename")
            .ok_or_else(|| CmdError::missing_argument("filename".to_string()))?;

        let selection = args.str_arg_or(1, "selection", "all");

        let state_arg = args.int_arg_or(2, "state", -1);

        let path = expand_path(filename);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Session formats — handled before anything else
        match ext.as_str() {
            "prs" => {
                use patinae_session::save_prs;
                let session = ctx.viewer.session();
                save_prs(session, &path).map_err(|e| CmdError::file_format(e.to_string()))?;
                ctx.print(&format!(" Saved session to \"{}\"", filename));
                return Ok(());
            }
            "pse" => {
                return Err(CmdError::execution(
                    "PSE format is read-only. Use .prs for saving sessions.",
                ));
            }
            "bcif" => {
                return Err(CmdError::execution(
                    "BinaryCIF writing is not supported. Save as .cif instead.",
                ));
            }
            "pml" => {
                let mut file = std::fs::File::create(&path)
                    .map_err(|e| CmdError::file_format(e.to_string()))?;
                use std::io::Write;
                writeln!(file, "# Command log")
                    .map_err(|e| CmdError::file_format(e.to_string()))?;
                let count = if let Some(history) = ctx.history() {
                    let mut n = 0;
                    for cmd in history.iter() {
                        writeln!(file, "{}", cmd)
                            .map_err(|e| CmdError::file_format(e.to_string()))?;
                        n += 1;
                    }
                    n
                } else {
                    0
                };
                ctx.print(&format!(" Saved {} commands to \"{}\"", count, filename));
                return Ok(());
            }
            _ => {}
        }

        let format = FileFormat::from_extension(&ext);
        if format == FileFormat::Unknown && ctx.format_handler(&ext).is_none() {
            return Err(CmdError::execution(format!(
                "Unknown file format: .{} (supported: pdb, cif, mol2, sdf, xyz, gro, prs + plugins)",
                ext
            )));
        }

        // Evaluate selection across all objects
        let results = evaluate_selection(ctx.viewer, selection)?;
        let matches: Vec<(String, _)> = results
            .into_iter()
            .filter(|(_, sel)| sel.count() > 0)
            .collect();

        if matches.is_empty() {
            return Err(CmdError::selection(format!(
                "No atoms match '{}'",
                selection
            )));
        }

        // Resolve state index: -1/0 = current, positive values are 1-based.
        let state_idx: Option<usize> = match state_arg {
            n if n <= 0 => None,
            n => Some((n - 1) as usize),
        };

        // Build filtered molecules
        let filtered: Vec<ObjectMolecule> = matches
            .iter()
            .filter_map(|(name, sel)| {
                let mol_obj = ctx.viewer.objects().get_molecule(name)?;
                Some(extract_molecule(mol_obj.molecule(), sel, name, state_idx))
            })
            .filter(|m| m.atom_count() > 0)
            .collect();

        if filtered.is_empty() {
            return Err(CmdError::execution(
                "No atoms to save after applying selection",
            ));
        }

        let total_atoms: usize = filtered.iter().map(|m| m.atom_count()).sum();

        // Write
        let format_name = if format != FileFormat::Unknown {
            // Built-in format
            patinae_io::write_all(&path, &filtered)
                .map_err(|e| CmdError::file_format(e.to_string()))?;
            format.name().to_string()
        } else {
            // Plugin-provided format
            let handler = ctx.format_handler(&ext).unwrap();
            let writer_fn = handler.writer.as_ref().ok_or_else(|| {
                CmdError::execution(format!(
                    "Format '{}' does not support writing",
                    handler.name
                ))
            })?;
            let file =
                std::fs::File::create(&path).map_err(|e| CmdError::file_format(e.to_string()))?;
            let buf_writer: Box<dyn std::io::Write> = Box::new(std::io::BufWriter::new(file));
            (writer_fn)(buf_writer, &filtered).map_err(CmdError::file_format)?;
            handler.name.clone()
        };

        ctx.print(&format!(
            " Saved {} atoms to \"{}\" ({} format)",
            total_atoms, filename, format_name
        ));

        Ok(())
    }
}

// ============================================================================
// png command
// ============================================================================

/// Save current view as PNG image
struct PngCommand;

impl Command for PngCommand {
    fn name(&self) -> &str {
        "png"
    }

    command_help! {
        CMD "png"
        DESCRIPTION [
            "saves a PNG format image file of the current display.",
        ]
        REQUIRED [
            { "filename", "string", "file path to be written" },
        ]
        OPTIONAL [
            { "width", "integer", "width in pixels", "current window width" },
            { "height", "integer", "height in pixels", "current window height" },
            { "dpi", "float", "dots-per-inch (not yet implemented)", "0" },
            { "quiet", "0/1", "suppress feedback", "0" },
        ]
        NOTES("NOTES") [
            "If only width or height is specified, the aspect ratio is preserved.",
            "Use the \"ray\" command (from the raytracer plugin) for ray-traced images.",
        ]
        EXAMPLES [
            "png ~/Desktop/screenshot.png",
            "png output.png, 1920, 1080",
            "png output.png, width=800",
            "png hires.png, 4096, 4096, quiet=1",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Path]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        // Get filename (required)
        let filename = args
            .str_arg(0, "filename")
            .ok_or_else(|| CmdError::missing_argument("filename".to_string()))?;

        // Get width (optional)
        let width = args.int_arg(1, "width").map(|v| v as u32);

        // Get height (optional)
        let height = args.int_arg(2, "height").map(|v| v as u32);

        // Get dpi (optional, not yet used)
        let _dpi = args.float_arg(3, "dpi");

        // Get quiet flag
        let quiet = args.bool_arg_or(4, "quiet", false);

        // Ensure the path ends with .png
        let path = expand_path(filename);
        let path = if path.extension().map(|e| e.to_ascii_lowercase()) != Some("png".into()) {
            path.with_extension("png")
        } else {
            path.to_path_buf()
        };

        if let Some(vp_img) = ctx.viewer.get_viewport_image() {
            // Export stored viewport image overlay
            #[cfg(feature = "native-io")]
            {
                use image::RgbaImage;

                let img = RgbaImage::from_raw(vp_img.width, vp_img.height, vp_img.data.clone())
                    .ok_or_else(|| {
                        CmdError::execution("Invalid viewport image data".to_string())
                    })?;

                img.save(&path)
                    .map_err(|e| CmdError::execution(format!("Failed to save PNG: {}", e)))?;

                if !quiet {
                    ctx.print(&format!(" ({}x{})", vp_img.width, vp_img.height));
                    ctx.print(&format!(" Saved viewport image to \"{}\"", path.display()));
                }
            }
            #[cfg(not(feature = "native-io"))]
            {
                let _ = vp_img;
                return Err(CmdError::execution(
                    "PNG export not available in this build".to_string(),
                ));
            }
        } else if let Some((w, h)) = ctx
            .viewer
            .save_viewport_gpu_image(&path)
            .map_err(|e| CmdError::execution(format!("Failed to save viewport PNG: {}", e)))?
        {
            if !quiet {
                ctx.print(&format!(" ({}x{})", w, h));
                ctx.print(&format!(" Saved viewport image to \"{}\"", path.display()));
            }
        } else {
            // Capture rasterized screenshot
            ctx.viewer
                .capture_png(&path, width, height)
                .map_err(|e| CmdError::execution(format!("Failed to capture PNG: {}", e)))?;

            if !quiet {
                let (w, h) = match (width, height) {
                    (Some(w), Some(h)) => (w, h),
                    _ => {
                        // Report actual captured dimensions
                        // For now just report the requested or window size
                        (width.unwrap_or(0), height.unwrap_or(0))
                    }
                };
                if w > 0 && h > 0 {
                    ctx.print(&format!(" ({}x{})", w, h));
                }
                ctx.print(&format!(" Saved \"{}\"", path.display()));
            }
        }

        Ok(())
    }
}

#[cfg(all(test, feature = "native-io"))]
mod tests {
    use crate::CommandExecutor;
    use patinae_scene::{Session, SessionAdapter, ViewportImage};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn png_exports_stored_viewport_image_overlay() {
        let mut session = Session::new();
        session.viewport_image = Some(ViewportImage {
            width: 2,
            height: 2,
            data: vec![
                255, 0, 0, 255, //
                0, 255, 0, 255, //
                0, 0, 255, 255, //
                255, 255, 255, 255,
            ],
        });

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "patinae-png-overlay-{}-{unique}.png",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        let mut needs_redraw = false;
        {
            let mut adapter = SessionAdapter {
                session: &mut session,
                render_context: None,
                default_size: (800, 600),
                needs_redraw: &mut needs_redraw,
                async_fetch_fn: None,
            };
            let mut executor = CommandExecutor::new();
            let path_arg = path
                .display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            let command = format!("png \"{path_arg}\", quiet=1");
            executor
                .do_with_options(&mut adapter, &command, true)
                .unwrap();
        }

        let bytes = fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(image::image_dimensions(&path).unwrap(), (2, 2));

        let _ = fs::remove_file(&path);
    }
}

// ============================================================================
// cd command
// ============================================================================

/// Change current working directory
struct CdCommand;

impl Command for CdCommand {
    fn name(&self) -> &str {
        "cd"
    }

    command_help! {
        CMD "cd"
        DESCRIPTION [
            "changes the current working directory.",
        ]
        REQUIRED []
        OPTIONAL [
            { "path", "string", "directory path", "home directory" },
        ]
        EXAMPLES [
            "cd /home/user/molecules",
            "cd ..",
            "cd",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Path]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let path = args.str_arg(0, "path");

        let target = if let Some(p) = path {
            expand_path(p)
        } else {
            // Default to home directory
            dirs_path_or_current()
        };

        env::set_current_dir(&target).map_err(|e| {
            CmdError::execution(format!("Cannot change to '{}': {}", target.display(), e))
        })?;

        ctx.print(&format!(" Changed to {}", target.display()));

        Ok(())
    }
}

/// Get home directory or current directory
fn dirs_path_or_current() -> std::path::PathBuf {
    env::var("HOME")
        .map(|h| Path::new(&h).to_path_buf())
        .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf()))
}

// ============================================================================
// pwd command
// ============================================================================

/// Print current working directory
struct PwdCommand;

impl Command for PwdCommand {
    fn name(&self) -> &str {
        "pwd"
    }

    command_help! {
        CMD "pwd"
        DESCRIPTION [
            "prints the current working directory.",
        ]
        REQUIRED []
        OPTIONAL []
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        let cwd = env::current_dir().map_err(|e| CmdError::execution(e.to_string()))?;

        ctx.print(&format!(" {}", cwd.display()));

        Ok(())
    }
}

// ============================================================================
// ls command
// ============================================================================

/// List directory contents
struct LsCommand;

impl Command for LsCommand {
    fn name(&self) -> &str {
        "ls"
    }

    fn aliases(&self) -> &[&str] {
        &["dir"]
    }

    command_help! {
        CMD "ls"
        DESCRIPTION [
            "lists the contents of a directory.",
        ]
        REQUIRED []
        OPTIONAL [
            { "path", "string", "directory path", "current directory" },
        ]
        EXAMPLES [
            "ls",
            "ls /home/user/molecules",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::Path]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let path_str = args.str_arg_or(0, "path", ".");

        let dir = expand_path(path_str);

        if !dir.is_dir() {
            return Err(CmdError::execution(format!(
                "'{}' is not a directory",
                path_str
            )));
        }

        let mut entries: Vec<String> = std::fs::read_dir(dir)
            .map_err(|e| CmdError::execution(e.to_string()))?
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    format!("{}/", name)
                } else {
                    name
                }
            })
            .collect();

        entries.sort();

        // Format in columns
        for entry in entries {
            ctx.print(&format!("   {}", entry));
        }

        Ok(())
    }
}

#[cfg(test)]
mod prs_tests {
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use flate2::write::GzEncoder;
    use flate2::Compression;
    use patinae_scene::{Session, SessionAdapter};

    use crate::{CommandExecutor, MessageKind};

    fn temp_prs_path(name: &str) -> (PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("patinae_cmd_prs_{}_{}", name, std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legacy.prs");
        (dir, path)
    }

    fn write_legacy_prs(path: &Path) {
        let data = rmp_serde::to_vec(&Session::new()).unwrap();
        let file = File::create(path).unwrap();
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(&data).unwrap();
        encoder.finish().unwrap();
    }

    #[test]
    fn load_legacy_prs_emits_warning_message() {
        let (dir, path) = temp_prs_path("legacy_warning");
        write_legacy_prs(&path);
        let mut session = Session::new();
        let mut needs_redraw = false;
        let mut adapter = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };

        let output = CommandExecutor::new()
            .do_with_options(
                &mut adapter,
                &format!("load {}", path.to_string_lossy()),
                false,
            )
            .unwrap();

        assert!(output.messages.iter().any(|message| {
            message.kind == MessageKind::Warning && message.text.contains("legacy PRS session")
        }));

        let _ = fs::remove_dir_all(&dir);
    }
}

// ============================================================================
// fetch command (optional, requires fetch feature)
// ============================================================================

#[cfg(any(feature = "fetch", feature = "fetch-async"))]
struct FetchCommand;

#[cfg(any(feature = "fetch", feature = "fetch-async"))]
impl Command for FetchCommand {
    fn name(&self) -> &str {
        "fetch"
    }

    command_help! {
        CMD "fetch"
        DESCRIPTION [
            "downloads a structure from the RCSB PDB.",
        ]
        REQUIRED [
            { "code", "string", "PDB ID (e.g., \"1ubq\")" },
        ]
        OPTIONAL [
            { "name", "string", "object name", "PDB ID" },
            { "type", "string", "file type to fetch", "cif" } => [
                "Supported: \"pdb\", \"cif\" (or \"mmcif\"), \"bcif\"",
            ],
            { "async", "0/1", "asynchronous fetch", "1" } => [
                "When 0, forces synchronous (blocking) fetch",
            ],
        ]
        EXAMPLES [
            "fetch 1ubq",
            "fetch 4hhb, name=hemoglobin",
            "fetch 1crn, type=pdb",
            "fetch 1igt, async=0",
        ]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[
            ArgHint::None,
            ArgHint::Object,
            ArgHint::Keywords(&["pdb", "cif", "mmcif", "bcif"]),
        ]
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let code = args
            .str_arg(0, "code")
            .ok_or_else(|| CmdError::missing_argument("code".to_string()))?;

        let name = args.str_arg_or(1, "name", code);

        // Parse the type argument (default: bcif)
        let format = args
            .str_arg(2, "type")
            .map(|s| match s.to_lowercase().as_str() {
                "pdb" => patinae_io::FetchFormat::Pdb,
                "cif" | "mmcif" => patinae_io::FetchFormat::Cif,
                "bcif" | "binarycif" => patinae_io::FetchFormat::Bcif,
                _ => patinae_io::FetchFormat::Cif,
            })
            .unwrap_or(patinae_io::FetchFormat::Cif);

        let bond_tolerance = ctx.viewer.settings().behavior.bonding_vdw_cutoff;
        let auto_dss = ctx.viewer.settings().behavior.auto_dss;
        let dss_algorithm = ctx.viewer.settings().behavior.dss_algorithm;
        // Parse async flag (default: true = non-blocking). Async requests
        // capture behavior settings at submission time for deterministic
        // completion even if settings change while the fetch is in flight.
        let use_async = args.get_named_bool_or("async", true);

        // Convert format to a command-layer code for host async APIs.
        let format_code = match format {
            patinae_io::FetchFormat::Pdb => FetchFormatCode::Pdb,
            patinae_io::FetchFormat::Cif => FetchFormatCode::Cif,
            patinae_io::FetchFormat::Bcif => FetchFormatCode::Bcif,
        };

        // Try async path first (GUI supports this)
        if use_async
            && ctx.submit_async_request(AsyncCommandRequest::Fetch(FetchRequest {
                code: code.to_string(),
                name: name.to_string(),
                format: format_code,
                bond_tolerance,
                auto_dss,
                dss_algorithm,
            }))
        {
            ctx.print(&format!(" Fetching {}...", code));
            return Ok(());
        }

        // Sync fallback for non-GUI viewers (headless, scripts, etc.)
        #[cfg(feature = "fetch")]
        {
            let mol = patinae_io::fetch_with_bond_tolerance(code, format, bond_tolerance)
                .map_err(|e| CmdError::file_format(e.to_string()))?;

            finalize_fetched_molecule(ctx.viewer, name, mol, auto_dss, dss_algorithm);
            ctx.print(&format!(" Fetched {} as \"{}\"", code, name));

            Ok(())
        }

        // No fetch implementation available
        #[cfg(not(feature = "fetch"))]
        {
            return Err(CmdError::execution(
                "Fetch not available: neither async nor sync fetch is enabled",
            ));
        }
    }
}

#[cfg(test)]
mod fetch_tests {
    #[cfg(any(feature = "fetch", feature = "fetch-async"))]
    mod fetch_async_requests {
        use patinae_scene::{Session, SessionAdapter};

        use crate::{AsyncCommandRequest, CommandExecutor, FetchFormatCode};

        fn with_adapter<T>(
            f: impl FnOnce(&mut CommandExecutor, &mut SessionAdapter<'_>) -> T,
        ) -> T {
            let mut session = Session::new();
            let mut needs_redraw = false;
            let mut adapter = SessionAdapter {
                session: &mut session,
                render_context: None,
                default_size: (800, 600),
                needs_redraw: &mut needs_redraw,
                async_fetch_fn: None,
            };
            let mut executor = CommandExecutor::new();
            f(&mut executor, &mut adapter)
        }

        #[test]
        fn fetch_submits_async_request_by_default() {
            let mut captured = Vec::new();
            let output = with_adapter(|executor, adapter| {
                let mut sink = |request| {
                    captured.push(request);
                    true
                };
                executor.do_with_async_sink(adapter, "fetch 1ubq", false, Some(&mut sink))
            })
            .expect("fetch command should be accepted by async sink");

            assert_eq!(captured.len(), 1);
            let AsyncCommandRequest::Fetch(request) = &captured[0];
            assert_eq!(request.code, "1ubq");
            assert_eq!(request.name, "1ubq");
            assert_eq!(request.format, FetchFormatCode::Cif);
            assert!(request.auto_dss);
            assert!(output
                .messages
                .iter()
                .any(|message| message.text == " Fetching 1ubq..."));
        }

        #[cfg(feature = "fetch")]
        #[test]
        fn fetch_async_zero_bypasses_async_sink() {
            let mut captured = Vec::new();
            let result = with_adapter(|executor, adapter| {
                let mut sink = |request| {
                    captured.push(request);
                    true
                };
                executor.do_with_async_sink(adapter, "fetch abcde, async=0", false, Some(&mut sink))
            });

            assert!(result.is_err(), "invalid PDB ID should fail in sync path");
            assert!(captured.is_empty());
        }
    }
}
