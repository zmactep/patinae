//! Streams atom rows from the live scene.
//!
//! The types in this module are shared by in-process panels and portable
//! plugin wire payloads. In-process callers can iterate borrowed atom rows,
//! while ABI callers receive owned chunks built from the same stream plan.

use patinae_mol::{Atom, AtomIndex, SecondaryStructure};
use patinae_scene::{MoleculeObject, ObjectRegistry};
use patinae_select::{self, SelectionOptions};
use serde::{Deserialize, Serialize};

use crate::component::SharedContext;

/// Default number of atom rows requested per stream chunk.
pub const DEFAULT_ATOM_STREAM_CHUNK_ROWS: usize = 4096;

/// Maximum rows returned in one atom stream chunk.
pub const MAX_ATOM_STREAM_CHUNK_ROWS: usize = 16_384;

/// Target encoded size for one atom stream chunk.
pub const ATOM_STREAM_CHUNK_TARGET_BYTES: usize = 8 * 1024 * 1024;

/// Describes which atoms a stream should visit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtomStreamScope {
    /// Visit every molecule object.
    All,
    /// Visit one molecule object.
    Object(String),
    /// Visit the listed molecule objects.
    Objects(Vec<String>),
    /// Visit atoms matching a selection expression.
    Selection(String),
}

/// Describes how a stream will use atom rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtomStreamMode {
    /// Rows will be read without mutation.
    Read,
    /// Rows may be used to produce atom mutations.
    Alter,
}

/// Atom row columns available to stream clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AtomColumn {
    /// Atom name.
    Name,
    /// Residue name.
    Resn,
    /// Integer residue id.
    Resv,
    /// PyMOL-compatible residue id alias.
    Resi,
    /// Chain id.
    Chain,
    /// Segment id.
    Segi,
    /// Alternate location id.
    Alt,
    /// Element symbol.
    Elem,
    /// B-factor.
    B,
    /// Occupancy.
    Q,
    /// Effective van der Waals radius.
    Vdw,
    /// Partial charge.
    PartialCharge,
    /// Formal charge.
    FormalCharge,
    /// Secondary-structure code.
    Ss,
    /// Base color index.
    Color,
    /// PDB record type.
    Type,
    /// Whether this atom is HETATM.
    Hetatm,
    /// One-based atom index.
    Index,
    /// PDB atom serial number.
    Id,
    /// Rank for ordering.
    Rank,
    /// Object/model name.
    Model,
    /// Displayed x coordinate.
    X,
    /// Displayed y coordinate.
    Y,
    /// Displayed z coordinate.
    Z,
}

impl AtomColumn {
    /// Returns the columns used by PyMOL-compatible `iterate` locals.
    pub fn pymol_locals() -> Vec<Self> {
        vec![
            Self::Name,
            Self::Resn,
            Self::Resv,
            Self::Resi,
            Self::Chain,
            Self::Segi,
            Self::Alt,
            Self::Elem,
            Self::B,
            Self::Q,
            Self::Vdw,
            Self::PartialCharge,
            Self::FormalCharge,
            Self::Ss,
            Self::Color,
            Self::Type,
            Self::Hetatm,
            Self::Index,
            Self::Id,
            Self::Rank,
            Self::Model,
            Self::X,
            Self::Y,
            Self::Z,
        ]
    }

    /// Returns the Python/PyMOL local name for this column.
    pub fn local_name(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Resn => "resn",
            Self::Resv => "resv",
            Self::Resi => "resi",
            Self::Chain => "chain",
            Self::Segi => "segi",
            Self::Alt => "alt",
            Self::Elem => "elem",
            Self::B => "b",
            Self::Q => "q",
            Self::Vdw => "vdw",
            Self::PartialCharge => "partial_charge",
            Self::FormalCharge => "formal_charge",
            Self::Ss => "ss",
            Self::Color => "color",
            Self::Type => "type",
            Self::Hetatm => "hetatm",
            Self::Index => "index",
            Self::Id => "ID",
            Self::Rank => "rank",
            Self::Model => "model",
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
        }
    }

    /// Returns whether `alter` may mutate this column.
    pub fn is_mutable(self) -> bool {
        matches!(
            self,
            Self::Name
                | Self::Resn
                | Self::Resv
                | Self::Resi
                | Self::Chain
                | Self::Segi
                | Self::Alt
                | Self::Elem
                | Self::B
                | Self::Q
                | Self::Vdw
                | Self::PartialCharge
                | Self::FormalCharge
                | Self::Ss
                | Self::Color
                | Self::Type
        )
    }
}

/// Portable value for one atom row column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AtomValue {
    /// No value is available.
    None,
    /// String value.
    Str(String),
    /// 32-bit float value.
    F32(f32),
    /// 32-bit integer value.
    I32(i32),
    /// 8-bit integer value.
    I8(i8),
    /// Boolean value.
    Bool(bool),
}

/// Stable row identity within an open stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomRowKey {
    /// Object/model name.
    pub object: String,
    /// Zero-based atom index in the owning object.
    pub atom_index: u32,
}

/// Owned atom row for ABI and worker handoff.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AtomRow {
    /// Hidden row identity.
    pub key: AtomRowKey,
    /// Requested column values.
    pub values: Vec<(AtomColumn, AtomValue)>,
}

impl AtomRow {
    /// Returns a streamed column value.
    pub fn get(&self, column: AtomColumn) -> Option<&AtomValue> {
        self.values
            .iter()
            .find_map(|(candidate, value)| (*candidate == column).then_some(value))
    }
}

/// Owned atom rows returned by one stream read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AtomChunk {
    /// Rows in stream order.
    pub rows: Vec<AtomRow>,
    /// True when the stream has no further rows.
    pub done: bool,
}

/// Atom stream request shared by Rust and ABI callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomStreamRequest {
    /// Atoms to visit.
    pub scope: AtomStreamScope,
    /// Intended row use.
    pub mode: AtomStreamMode,
    /// Columns to include in owned rows.
    pub columns: Vec<AtomColumn>,
    /// Preferred chunk size.
    pub chunk_size: usize,
}

impl AtomStreamRequest {
    /// Builds a read stream over `scope`.
    pub fn new(scope: AtomStreamScope) -> Self {
        Self {
            scope,
            mode: AtomStreamMode::Read,
            columns: AtomColumn::pymol_locals(),
            chunk_size: DEFAULT_ATOM_STREAM_CHUNK_ROWS,
        }
    }

    /// Returns a bounded chunk size.
    pub fn bounded_chunk_size(&self) -> usize {
        self.chunk_size.clamp(1, MAX_ATOM_STREAM_CHUNK_ROWS)
    }
}

/// One selected object inside an atom stream plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomStreamObjectPlan {
    /// Object/model name.
    pub object: String,
    /// Atom count observed when the stream opened.
    pub atom_count: usize,
    /// Atom indices selected in this object.
    pub indices: AtomStreamIndices,
}

impl AtomStreamObjectPlan {
    fn len(&self) -> usize {
        self.indices.len(self.atom_count)
    }
}

/// Atom index set for one stream object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AtomStreamIndices {
    /// Every atom in the object.
    All,
    /// Explicit zero-based atom indices.
    Selected(Vec<u32>),
}

impl AtomStreamIndices {
    fn len(&self, atom_count: usize) -> usize {
        match self {
            Self::All => atom_count,
            Self::Selected(indices) => indices.len(),
        }
    }

    fn get(&self, atom_count: usize, position: usize) -> Option<u32> {
        match self {
            Self::All => (position < atom_count).then_some(position as u32),
            Self::Selected(indices) => indices.get(position).copied(),
        }
    }
}

/// Reusable atom stream plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomStreamPlan {
    /// Requested stream mode.
    pub mode: AtomStreamMode,
    /// Requested output columns.
    pub columns: Vec<AtomColumn>,
    /// Selected objects.
    pub objects: Vec<AtomStreamObjectPlan>,
    /// Total row count.
    pub total_count: usize,
}

impl AtomStreamPlan {
    /// Opens a stream plan from the current shared context.
    ///
    /// # Errors
    /// Returns an error when a requested object is missing, is not a molecule,
    /// or when selection parsing/evaluation fails.
    pub fn open(ctx: &SharedContext<'_>, request: &AtomStreamRequest) -> Result<Self, String> {
        let objects = match &request.scope {
            AtomStreamScope::All => all_molecule_plans(ctx.registry),
            AtomStreamScope::Object(name) => vec![object_plan(ctx.registry, name)?],
            AtomStreamScope::Objects(names) => names
                .iter()
                .map(|name| object_plan(ctx.registry, name))
                .collect::<Result<Vec<_>, _>>()?,
            AtomStreamScope::Selection(selection) => selection_plans(ctx, selection)?,
        };
        let total_count = objects.iter().map(AtomStreamObjectPlan::len).sum();
        Ok(Self {
            mode: request.mode,
            columns: request.columns.clone(),
            objects,
            total_count,
        })
    }

    /// Returns an iterator over borrowed atom rows.
    pub fn iter<'a>(&self, registry: &'a ObjectRegistry) -> AtomStreamRows<'a> {
        AtomStreamRows {
            registry,
            plan: self.clone(),
            object_pos: 0,
            atom_pos: 0,
        }
    }

    /// Builds one owned chunk from this plan.
    ///
    /// # Errors
    /// Returns a stale-stream error when objects disappear or atom counts change.
    pub fn chunk(
        &self,
        ctx: &SharedContext<'_>,
        start: usize,
        max_rows: usize,
    ) -> Result<AtomChunk, String> {
        let mut rows = Vec::new();
        if start >= self.total_count || max_rows == 0 {
            return Ok(AtomChunk { rows, done: true });
        }

        let mut offset = 0usize;
        for object in &self.objects {
            let object_len = object.len();
            if start >= offset + object_len {
                offset += object_len;
                continue;
            }

            let molecule = checked_molecule(ctx.registry, object)?;
            let mut inner = start.saturating_sub(offset);
            while inner < object_len && rows.len() < max_rows {
                let atom_index = object
                    .indices
                    .get(object.atom_count, inner)
                    .ok_or_else(|| "atom stream plan contained an invalid index".to_string())?;
                let atom = molecule
                    .molecule()
                    .get_atom(AtomIndex(atom_index))
                    .ok_or_else(|| stale_stream_error(&object.object))?;
                let coord = molecule
                    .display_coord(AtomIndex(atom_index))
                    .map(|coord| [coord.x, coord.y, coord.z]);
                rows.push(owned_row(
                    &object.object,
                    atom_index,
                    atom,
                    coord,
                    &self.columns,
                ));
                inner += 1;
            }

            if rows.len() >= max_rows {
                break;
            }
            offset += object_len;
        }

        let done = start + rows.len() >= self.total_count;
        Ok(AtomChunk { rows, done })
    }
}

/// Borrowed atom row for in-process callers.
pub struct BorrowedAtomRow<'a> {
    /// Object/model name.
    pub object: String,
    /// Zero-based atom index.
    pub atom_index: u32,
    /// Borrowed atom.
    pub atom: &'a Atom,
    /// Displayed coordinates, when available.
    pub coord: Option<[f32; 3]>,
}

impl BorrowedAtomRow<'_> {
    /// Converts this borrowed row into an owned row.
    pub fn to_owned_row(&self, columns: &[AtomColumn]) -> AtomRow {
        owned_row(
            &self.object,
            self.atom_index,
            self.atom,
            self.coord,
            columns,
        )
    }
}

/// Borrowed atom stream iterator.
pub struct AtomStreamRows<'a> {
    registry: &'a ObjectRegistry,
    plan: AtomStreamPlan,
    object_pos: usize,
    atom_pos: usize,
}

impl<'a> Iterator for AtomStreamRows<'a> {
    type Item = BorrowedAtomRow<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(object) = self.plan.objects.get(self.object_pos) {
            let object_len = object.len();
            if self.atom_pos >= object_len {
                self.object_pos += 1;
                self.atom_pos = 0;
                continue;
            }

            let atom_index = object.indices.get(object.atom_count, self.atom_pos)?;
            self.atom_pos += 1;
            let molecule = self.registry.get_molecule(&object.object)?;
            let atom = molecule.molecule().get_atom(AtomIndex(atom_index))?;
            let coord = molecule
                .display_coord(AtomIndex(atom_index))
                .map(|coord| [coord.x, coord.y, coord.z]);
            return Some(BorrowedAtomRow {
                object: object.object.clone(),
                atom_index,
                atom,
                coord,
            });
        }
        None
    }
}

fn all_molecule_plans(registry: &ObjectRegistry) -> Vec<AtomStreamObjectPlan> {
    registry
        .names()
        .filter_map(|name| {
            let molecule = registry.get_molecule(name)?;
            Some(AtomStreamObjectPlan {
                object: name.to_string(),
                atom_count: molecule.molecule().atom_count(),
                indices: AtomStreamIndices::All,
            })
        })
        .collect()
}

fn object_plan(registry: &ObjectRegistry, name: &str) -> Result<AtomStreamObjectPlan, String> {
    let object = registry
        .get(name)
        .ok_or_else(|| format!("object '{name}' not found"))?;
    let molecule = registry
        .get_molecule(name)
        .ok_or_else(|| format!("object '{}' is a {}", name, object.object_type()))?;
    Ok(AtomStreamObjectPlan {
        object: name.to_string(),
        atom_count: molecule.molecule().atom_count(),
        indices: AtomStreamIndices::All,
    })
}

fn selection_plans(
    ctx: &SharedContext<'_>,
    selection: &str,
) -> Result<Vec<AtomStreamObjectPlan>, String> {
    let expr = patinae_select::parse(selection).map_err(|error| error.to_string())?;
    let object_names: Vec<String> = ctx.registry.names().map(str::to_string).collect();
    let mut objects = Vec::new();

    for name in &object_names {
        let Some(molecule) = ctx.registry.get_molecule(name) else {
            continue;
        };
        let mol = molecule.molecule();
        let eval = ctx.selections.build_eval_context(
            mol,
            molecule.display_state(),
            name,
            &object_names,
            SelectionOptions::default(),
        );
        let selection =
            patinae_select::evaluate(&expr, &eval).map_err(|error| error.to_string())?;
        let indices: Vec<u32> = selection
            .raw_indices()
            .filter_map(|index| u32::try_from(index).ok())
            .collect();
        if !indices.is_empty() {
            objects.push(AtomStreamObjectPlan {
                object: name.clone(),
                atom_count: mol.atom_count(),
                indices: AtomStreamIndices::Selected(indices),
            });
        }
    }

    Ok(objects)
}

fn checked_molecule<'a>(
    registry: &'a ObjectRegistry,
    object: &AtomStreamObjectPlan,
) -> Result<&'a MoleculeObject, String> {
    let molecule = registry
        .get_molecule(&object.object)
        .ok_or_else(|| stale_stream_error(&object.object))?;
    if molecule.molecule().atom_count() != object.atom_count {
        return Err(stale_stream_error(&object.object));
    }
    Ok(molecule)
}

fn owned_row(
    object: &str,
    atom_index: u32,
    atom: &Atom,
    coord: Option<[f32; 3]>,
    columns: &[AtomColumn],
) -> AtomRow {
    AtomRow {
        key: AtomRowKey {
            object: object.to_string(),
            atom_index,
        },
        values: columns
            .iter()
            .copied()
            .map(|column| (column, atom_value(column, object, atom_index, atom, coord)))
            .collect(),
    }
}

fn atom_value(
    column: AtomColumn,
    object: &str,
    atom_index: u32,
    atom: &Atom,
    coord: Option<[f32; 3]>,
) -> AtomValue {
    match column {
        AtomColumn::Name => AtomValue::Str(atom.name.to_string()),
        AtomColumn::Resn => AtomValue::Str(atom.residue.resn.clone()),
        AtomColumn::Resv | AtomColumn::Resi => AtomValue::I32(atom.residue.resv),
        AtomColumn::Chain => AtomValue::Str(atom.residue.chain.clone()),
        AtomColumn::Segi => AtomValue::Str(atom.residue.segi.clone()),
        AtomColumn::Alt => AtomValue::Str(atom.alt.to_string()),
        AtomColumn::Elem => AtomValue::Str(atom.element.symbol().to_string()),
        AtomColumn::B => AtomValue::F32(atom.b_factor),
        AtomColumn::Q => AtomValue::F32(atom.occupancy),
        AtomColumn::Vdw => AtomValue::F32(atom.effective_vdw()),
        AtomColumn::PartialCharge => AtomValue::F32(atom.partial_charge),
        AtomColumn::FormalCharge => AtomValue::I8(atom.formal_charge),
        AtomColumn::Ss => AtomValue::Str(ss_to_str(atom.ss_type).to_string()),
        AtomColumn::Color => AtomValue::I32(atom.repr.colors.base),
        AtomColumn::Type => {
            AtomValue::Str(if atom.state.hetatm { "HETATM" } else { "ATOM" }.into())
        }
        AtomColumn::Hetatm => AtomValue::Bool(atom.state.hetatm),
        AtomColumn::Index => AtomValue::I32(atom_index as i32 + 1),
        AtomColumn::Id => AtomValue::I32(atom.id),
        AtomColumn::Rank => AtomValue::I32(atom.rank),
        AtomColumn::Model => AtomValue::Str(object.to_string()),
        AtomColumn::X => coord.map_or(AtomValue::None, |coord| AtomValue::F32(coord[0])),
        AtomColumn::Y => coord.map_or(AtomValue::None, |coord| AtomValue::F32(coord[1])),
        AtomColumn::Z => coord.map_or(AtomValue::None, |coord| AtomValue::F32(coord[2])),
    }
}

fn ss_to_str(ss: SecondaryStructure) -> &'static str {
    match ss {
        SecondaryStructure::Helix | SecondaryStructure::Helix310 | SecondaryStructure::HelixPi => {
            "H"
        }
        SecondaryStructure::Sheet => "S",
        _ => "L",
    }
}

fn stale_stream_error(object: &str) -> String {
    format!("atom stream for object '{object}' is stale because scene topology changed")
}

#[cfg(test)]
mod tests {
    use lin_alg::f32::Vec3;
    use patinae_cmd::CommandRegistry;
    use patinae_mol::{Atom, CoordSet, ObjectMolecule};
    use patinae_scene::Session;

    use super::*;
    use crate::component::SharedContext;

    fn test_molecule(name: &str) -> ObjectMolecule {
        let mut mol = ObjectMolecule::new(name);
        let mut ca = Atom {
            name: "CA".into(),
            ..Atom::default()
        };
        ca.residue =
            std::sync::Arc::new(patinae_mol::AtomResidue::from_parts("A", "ALA", 1, ' ', ""));
        mol.add_atom(ca);
        let mut cb = Atom {
            name: "CB".into(),
            ..Atom::default()
        };
        cb.residue =
            std::sync::Arc::new(patinae_mol::AtomResidue::from_parts("A", "GLY", 2, ' ', ""));
        mol.add_atom(cb);
        let mut coords = CoordSet::new();
        coords.add_coord(AtomIndex(0), Vec3::new(1.0, 2.0, 3.0));
        coords.add_coord(AtomIndex(1), Vec3::new(4.0, 5.0, 6.0));
        mol.add_coord_set(coords);
        mol
    }

    struct Fixture {
        session: Session,
        command_registry: CommandRegistry,
        command_names: Vec<String>,
    }

    impl Fixture {
        fn new() -> Self {
            let mut session = Session::new();
            session
                .registry
                .add(patinae_scene::MoleculeObject::new(test_molecule("obj")));
            Self {
                session,
                command_registry: CommandRegistry::new(),
                command_names: Vec::new(),
            }
        }

        fn shared(&self) -> SharedContext<'_> {
            SharedContext {
                registry: &self.session.registry,
                camera: &self.session.camera,
                selections: &self.session.selections,
                recent_atoms: &self.session.recent_atoms,
                named_palette: &self.session.named_palette,
                movie: &self.session.movie,
                settings: &self.session.settings,
                clear_color: self.session.clear_color,
                gpu_device: None,
                gpu_queue: None,
                scene_generation: 0,
                viewport_image: self.session.viewport_image.as_ref(),
                command_names: &self.command_names,
                command_registry: &self.command_registry,
                setting_names: &[],
                dynamic_settings: None,
            }
        }
    }

    #[test]
    fn all_scope_streams_displayed_coordinates() {
        let fixture = Fixture::new();
        let request = AtomStreamRequest {
            scope: AtomStreamScope::All,
            mode: AtomStreamMode::Read,
            columns: vec![AtomColumn::Name, AtomColumn::X, AtomColumn::Model],
            chunk_size: 16,
        };

        let plan = AtomStreamPlan::open(&fixture.shared(), &request).unwrap();
        let chunk = plan.chunk(&fixture.shared(), 0, 16).unwrap();

        assert_eq!(chunk.rows.len(), 2);
        assert_eq!(
            chunk.rows[0].get(AtomColumn::Name),
            Some(&AtomValue::Str("CA".to_string()))
        );
        assert_eq!(chunk.rows[0].get(AtomColumn::X), Some(&AtomValue::F32(1.0)));
        assert_eq!(
            chunk.rows[0].get(AtomColumn::Model),
            Some(&AtomValue::Str("obj".to_string()))
        );
        assert!(chunk.done);
    }

    #[test]
    fn selection_scope_preserves_selected_order() {
        let fixture = Fixture::new();
        let request = AtomStreamRequest {
            scope: AtomStreamScope::Selection("name CB".to_string()),
            mode: AtomStreamMode::Read,
            columns: vec![AtomColumn::Index, AtomColumn::Name],
            chunk_size: 16,
        };

        let plan = AtomStreamPlan::open(&fixture.shared(), &request).unwrap();
        let chunk = plan.chunk(&fixture.shared(), 0, 16).unwrap();

        assert_eq!(chunk.rows.len(), 1);
        assert_eq!(chunk.rows[0].key.atom_index, 1);
        assert_eq!(
            chunk.rows[0].get(AtomColumn::Index),
            Some(&AtomValue::I32(2))
        );
    }

    #[test]
    fn requested_columns_are_filtered() {
        let fixture = Fixture::new();
        let request = AtomStreamRequest {
            scope: AtomStreamScope::Object("obj".to_string()),
            mode: AtomStreamMode::Read,
            columns: vec![AtomColumn::Name],
            chunk_size: 16,
        };

        let plan = AtomStreamPlan::open(&fixture.shared(), &request).unwrap();
        let chunk = plan.chunk(&fixture.shared(), 0, 1).unwrap();

        assert_eq!(chunk.rows[0].values.len(), 1);
        assert_eq!(chunk.rows[0].values[0].0, AtomColumn::Name);
        assert!(!chunk.done);
    }

    #[test]
    fn stale_topology_returns_error() {
        let mut fixture = Fixture::new();
        let request = AtomStreamRequest::new(AtomStreamScope::Object("obj".to_string()));
        let plan = AtomStreamPlan::open(&fixture.shared(), &request).unwrap();
        fixture
            .session
            .registry
            .get_molecule_mut("obj")
            .unwrap()
            .molecule_mut()
            .add_atom(Atom::default());

        let err = plan.chunk(&fixture.shared(), 0, 1).unwrap_err();

        assert!(err.contains("stale"));
    }
}
