//! Selection expression evaluator
//!
//! Evaluates parsed selection expressions against molecules in a context.

use lin_alg::f32::Vec3;
use patinae_mol::{
    spatial::SpatialGrid, three_to_one, Atom, AtomFlags, AtomIndex, SecondaryStructure,
};

use crate::ast::{CompareOp, MacroSpec, PropertyValue, SelectionExpr};
use crate::context::EvalContext;
use crate::error::{EvalError, EvalResult};
use crate::pattern::Pattern;
use crate::result::SelectionResult;

/// Evaluate a selection expression against a context
pub fn evaluate(expr: &SelectionExpr, ctx: &EvalContext) -> EvalResult<SelectionResult> {
    if ctx.molecule_count() == 0 {
        return Err(EvalError::NoMolecules);
    }

    eval_expr(expr, ctx)
}

/// Internal evaluation dispatcher
fn eval_expr(expr: &SelectionExpr, ctx: &EvalContext) -> EvalResult<SelectionResult> {
    match expr {
        // Logical operators
        SelectionExpr::And(left, right) => {
            let l = eval_expr(left, ctx)?;
            let r = eval_expr(right, ctx)?;
            Ok(l.intersection(&r))
        }
        SelectionExpr::Or(left, right) => {
            let l = eval_expr(left, ctx)?;
            let r = eval_expr(right, ctx)?;
            Ok(l.union(&r))
        }
        SelectionExpr::Not(inner) => {
            let result = eval_expr(inner, ctx)?;
            Ok(result.complement())
        }

        // Property selectors
        SelectionExpr::Name(pattern) => eval_property(ctx, |atom, _| {
            pattern.matches(&atom.name, !ctx.options.ignore_case)
        }),
        SelectionExpr::Resn(pattern) => eval_property(ctx, |atom, _| {
            pattern.matches(&atom.residue.resn, !ctx.options.ignore_case)
        }),
        SelectionExpr::Resi(spec) => eval_property(ctx, |atom, _| {
            spec.matches(atom.residue.resv, atom.residue.inscode)
        }),
        SelectionExpr::Chain(pattern) => eval_property(ctx, |atom, _| {
            pattern.matches(&atom.residue.chain, !ctx.options.ignore_case_chain)
        }),
        SelectionExpr::Segi(pattern) => eval_property(ctx, |atom, _| {
            pattern.matches(&atom.residue.segi, !ctx.options.ignore_case_chain)
        }),
        SelectionExpr::Elem(pattern) => eval_property(ctx, |atom, _| {
            pattern.matches(atom.element.symbol(), !ctx.options.ignore_case)
        }),
        SelectionExpr::Alt(pattern) => eval_property(ctx, |atom, _| {
            pattern.matches(&atom.alt.to_string(), !ctx.options.ignore_case)
        }),
        SelectionExpr::Model(pattern) => eval_model(ctx, pattern),
        SelectionExpr::Index(spec) => {
            eval_property(ctx, |_, info| spec.matches(info.local_idx as i32))
        }
        SelectionExpr::Id(spec) => eval_property(ctx, |atom, _| spec.matches(atom.id)),
        SelectionExpr::Rank(spec) => eval_property(ctx, |atom, _| spec.matches(atom.rank)),
        SelectionExpr::SecondaryStructure(pattern) => eval_property(ctx, |atom, _| {
            let ss_str = match atom.ss_type {
                SecondaryStructure::Helix => "H",
                SecondaryStructure::Sheet => "S",
                SecondaryStructure::Loop => "L",
                SecondaryStructure::Turn => "T",
                SecondaryStructure::Helix310 => "G",
                SecondaryStructure::HelixPi => "I",
                SecondaryStructure::Bend => "B",
                SecondaryStructure::NucleicRibbon => "N",
            };
            pattern.matches(ss_str, !ctx.options.ignore_case)
        }),
        SelectionExpr::State(spec) => {
            eval_property(ctx, |atom, _| spec.matches(atom.discrete_state))
        }
        SelectionExpr::Flag(spec) => eval_property(ctx, |atom, _| {
            // Check if any flag in the spec matches
            spec.items.iter().any(|item| match item {
                crate::pattern::IntItem::Single(flag) => {
                    (atom.state.flags.bits() & (1 << flag)) != 0
                }
                crate::pattern::IntItem::Range(start, end) => {
                    (*start..=*end).any(|flag| (atom.state.flags.bits() & (1 << flag)) != 0)
                }
            })
        }),
        SelectionExpr::Rep(pattern) => eval_property(ctx, |atom, _| {
            // Map representation names to their bitmask values
            use patinae_mol::RepMask;
            let rep_map: &[(&str, RepMask)] = &[
                ("lines", RepMask::LINES),
                ("line", RepMask::LINES),
                ("spheres", RepMask::SPHERES),
                ("sphere", RepMask::SPHERES),
                ("surface", RepMask::SURFACE),
                ("surf", RepMask::SURFACE),
                ("labels", RepMask::LABELS),
                ("label", RepMask::LABELS),
                ("nonbonded", RepMask::NONBONDED),
                ("nb_spheres", RepMask::NONBONDED),
                ("cartoon", RepMask::CARTOON),
                ("cart", RepMask::CARTOON),
                ("ribbon", RepMask::RIBBON),
                ("ribb", RepMask::RIBBON),
                ("sticks", RepMask::STICKS),
                ("stick", RepMask::STICKS),
                ("mesh", RepMask::MESH),
                ("dots", RepMask::DOTS),
                ("dot", RepMask::DOTS),
            ];
            // Check if the atom has the specific representation visible
            rep_map.iter().any(|(rep_name, rep_mask)| {
                pattern.matches(rep_name, !ctx.options.ignore_case)
                    && atom.repr.visible_reps.is_visible(*rep_mask)
            })
        }),
        SelectionExpr::Color(pattern) => eval_property(ctx, |atom, _| {
            pattern.matches(&atom.repr.colors.base.to_string(), !ctx.options.ignore_case)
        }),
        SelectionExpr::CartoonColor(pattern) => eval_property(ctx, |atom, _| {
            pattern.matches(
                &atom.repr.colors.cartoon_or_base().to_string(),
                !ctx.options.ignore_case,
            )
        }),
        SelectionExpr::RibbonColor(pattern) => eval_property(ctx, |atom, _| {
            pattern.matches(
                &atom.repr.colors.ribbon_or_base().to_string(),
                !ctx.options.ignore_case,
            )
        }),
        SelectionExpr::Label(pattern) => eval_property(ctx, |_, info| {
            ctx.atom_labels(info.global_idx)
                .any(|label| pattern.matches(label, !ctx.options.ignore_case))
        }),
        SelectionExpr::Custom(pattern) => eval_property(ctx, |atom, _| {
            // Custom properties would need additional support
            pattern.matches(&atom.repr.text_type, !ctx.options.ignore_case)
        }),
        SelectionExpr::TextType(pattern) => eval_property(ctx, |atom, _| {
            pattern.matches(&atom.repr.text_type, !ctx.options.ignore_case)
        }),
        SelectionExpr::NumericType(pattern) => eval_property(ctx, |atom, _| {
            // Numeric type is typically an integer
            pattern.matches(&atom.repr.text_type, !ctx.options.ignore_case)
        }),
        SelectionExpr::Stereo(pattern) => eval_property(ctx, |atom, _| {
            let stereo_str = format!("{:?}", atom.stereo);
            pattern.matches(&stereo_str, !ctx.options.ignore_case)
        }),
        SelectionExpr::PepSeq(pattern) => eval_pepseq(ctx, pattern),

        // Numeric comparisons
        SelectionExpr::BFactor(op, val) => {
            eval_property(ctx, |atom, _| op.compare_f32(atom.b_factor, *val))
        }
        SelectionExpr::Occupancy(op, val) => {
            eval_property(ctx, |atom, _| op.compare_f32(atom.occupancy, *val))
        }
        SelectionExpr::PartialCharge(op, val) => {
            eval_property(ctx, |atom, _| op.compare_f32(atom.partial_charge, *val))
        }
        SelectionExpr::FormalCharge(op, val) => eval_property(ctx, |atom, _| {
            op.compare_i32(atom.formal_charge as i32, *val)
        }),
        SelectionExpr::X(op, val) => eval_coord(ctx, |coord| op.compare_f32(coord.x, *val)),
        SelectionExpr::Y(op, val) => eval_coord(ctx, |coord| op.compare_f32(coord.y, *val)),
        SelectionExpr::Z(op, val) => eval_coord(ctx, |coord| op.compare_f32(coord.z, *val)),

        // Property access
        SelectionExpr::Property(name, op, value) => eval_generic_property(ctx, name, *op, value),

        // Special selections
        SelectionExpr::All => Ok(SelectionResult::all(ctx.total_atoms())),
        SelectionExpr::None => Ok(SelectionResult::none(ctx.total_atoms())),
        SelectionExpr::Visible => eval_property(ctx, |atom, _| atom.repr.visible_reps.0 != 0),
        SelectionExpr::Enabled => eval_property(ctx, |atom, _| !atom.repr.masked),
        SelectionExpr::Hydrogens => eval_property(ctx, |atom, _| atom.element.is_hydrogen()),
        SelectionExpr::Hetatm => eval_property(ctx, |atom, _| atom.state.hetatm),
        SelectionExpr::Bonded => eval_property(ctx, |atom, _| atom.state.bonded),
        SelectionExpr::Polymer => {
            eval_property(ctx, |atom, _| atom.state.flags.contains(AtomFlags::POLYMER))
        }
        SelectionExpr::PolymerProtein => {
            eval_property(ctx, |atom, _| atom.state.flags.contains(AtomFlags::PROTEIN))
        }
        SelectionExpr::PolymerNucleic => {
            eval_property(ctx, |atom, _| atom.state.flags.contains(AtomFlags::NUCLEIC))
        }
        SelectionExpr::Organic => {
            eval_property(ctx, |atom, _| atom.state.flags.contains(AtomFlags::ORGANIC))
        }
        SelectionExpr::Inorganic => eval_property(ctx, |atom, _| {
            atom.state.flags.contains(AtomFlags::INORGANIC)
        }),
        SelectionExpr::Solvent => {
            eval_property(ctx, |atom, _| atom.state.flags.contains(AtomFlags::SOLVENT))
        }
        SelectionExpr::Metals => eval_property(ctx, |atom, _| atom.element.is_metal()),
        SelectionExpr::Backbone => eval_property(ctx, |atom, _| atom.is_backbone()),
        SelectionExpr::Sidechain => eval_property(ctx, |atom, _| atom.is_sidechain()),
        SelectionExpr::Donors => eval_property(ctx, |atom, _| atom.state.hb_donor),
        SelectionExpr::Acceptors => eval_property(ctx, |atom, _| atom.state.hb_acceptor),
        SelectionExpr::Delocalized => eval_property(ctx, |_atom, _| {
            // Simplified: check if atom is in aromatic system
            // In reality, this needs bond analysis
            false
        }),
        SelectionExpr::Fixed => {
            eval_property(ctx, |atom, _| atom.state.flags.contains(AtomFlags::FIX))
        }
        SelectionExpr::Restrained => eval_property(ctx, |atom, _| {
            atom.state.flags.contains(AtomFlags::RESTRAIN)
        }),
        SelectionExpr::Masked => eval_property(ctx, |atom, _| atom.repr.masked),
        SelectionExpr::Protected => {
            eval_property(ctx, |atom, _| atom.state.flags.contains(AtomFlags::EXCLUDE))
        }
        SelectionExpr::Present => eval_present(ctx),
        SelectionExpr::Guide => {
            eval_property(ctx, |atom, _| atom.state.flags.contains(AtomFlags::GUIDE))
        }
        SelectionExpr::Origin => {
            // Origin is a special point, not atom selection
            // Return empty for now - needs special handling in distance ops
            Ok(SelectionResult::none(ctx.total_atoms()))
        }
        SelectionExpr::Center => {
            // Center is a special point, not atom selection
            Ok(SelectionResult::none(ctx.total_atoms()))
        }

        // Distance-based operators
        SelectionExpr::Around(dist, inner) => eval_around(ctx, *dist, inner),
        SelectionExpr::Expand(dist, inner) => eval_expand(ctx, *dist, inner),
        SelectionExpr::Extend(n, inner) => eval_extend(ctx, *n, inner),
        SelectionExpr::Gap(dist, inner) => eval_gap(ctx, *dist, inner),
        SelectionExpr::Within(dist, source, target) => eval_within(ctx, *dist, source, target),
        SelectionExpr::Beyond(dist, source, target) => eval_beyond(ctx, *dist, source, target),
        SelectionExpr::NearTo(dist, source, target) => eval_near_to(ctx, *dist, source, target),

        // Grouping operators
        SelectionExpr::ByRes(inner) => eval_byres(ctx, inner),
        SelectionExpr::ByChain(inner) => eval_bychain(ctx, inner),
        SelectionExpr::ByObject(inner) => eval_byobject(ctx, inner),
        SelectionExpr::ByMolecule(inner) => eval_bymolecule(ctx, inner),
        SelectionExpr::BySegment(inner) => eval_bysegment(ctx, inner),
        SelectionExpr::ByFragment(inner) => eval_byfragment(ctx, inner),
        SelectionExpr::ByCAlpha(inner) => eval_bycalpha(ctx, inner),
        SelectionExpr::ByRing(inner) => eval_byring(ctx, inner),
        SelectionExpr::ByCell(inner) => eval_bycell(ctx, inner),

        // Neighbor operators
        SelectionExpr::Neighbor(inner) => eval_neighbor(ctx, inner),
        SelectionExpr::BoundTo(inner) => eval_bound_to(ctx, inner),

        // Position operators
        SelectionExpr::First(inner) => eval_first(ctx, inner),
        SelectionExpr::Last(inner) => eval_last(ctx, inner),

        // Set operators
        SelectionExpr::Like(left, right) => eval_like(ctx, left, right),
        SelectionExpr::In(left, right) => eval_in(ctx, left, right),

        // References
        SelectionExpr::Selection(name) => {
            // First try as named selection
            if let Some(result) = ctx.get_selection(name) {
                return Ok(result.clone());
            }
            // Fall back to model/object name matching (like "model name")
            let pattern = Pattern::Exact(name.clone());
            let result = eval_model(ctx, &pattern)?;
            if result.count() > 0 {
                Ok(result)
            } else {
                // Neither named selection nor model found with atoms
                Err(EvalError::SelectionNotFound(name.clone()))
            }
        }
        SelectionExpr::Macro(spec) => eval_macro(ctx, spec),
    }
}

/// Information about an atom during evaluation
struct AtomInfo {
    global_idx: usize,
    local_idx: usize,
    mol_idx: usize,
}

/// Evaluate a property-based selection
fn eval_property<F>(ctx: &EvalContext, predicate: F) -> EvalResult<SelectionResult>
where
    F: Fn(&Atom, &AtomInfo) -> bool,
{
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol_idx, (mol, offset)) in ctx.molecules_with_offsets().enumerate() {
        for (local_idx, atom) in mol.atoms().enumerate() {
            let global_idx = offset + local_idx;
            let info = AtomInfo {
                global_idx,
                local_idx,
                mol_idx,
            };
            if predicate(atom, &info) {
                result.set_index(global_idx);
            }
        }
    }

    Ok(result)
}

/// Evaluate a coordinate-based selection
fn eval_coord<F>(ctx: &EvalContext, predicate: F) -> EvalResult<SelectionResult>
where
    F: Fn(Vec3) -> bool,
{
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        let state = ctx.effective_state(mol);
        if let Some(cs) = mol.get_coord_set(state) {
            for (local_idx, _atom) in mol.atoms().enumerate() {
                if let Some(coord) = cs.get_atom_coord(AtomIndex(local_idx as u32)) {
                    if predicate(coord) {
                        result.set_index(offset + local_idx);
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Evaluate model/object selection
fn eval_model(ctx: &EvalContext, pattern: &Pattern) -> EvalResult<SelectionResult> {
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        if pattern.matches(&mol.name, !ctx.options.ignore_case) {
            for local_idx in 0..mol.atom_count() {
                result.set_index(offset + local_idx);
            }
        }
    }

    Ok(result)
}

/// Evaluate peptide sequence selection (single-letter amino acid codes)
fn eval_pepseq(ctx: &EvalContext, pattern: &Pattern) -> EvalResult<SelectionResult> {
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        for (local_idx, atom) in mol.atoms().enumerate() {
            if let Some(code) = three_to_one(&atom.residue.resn) {
                if pattern.matches(&code.to_string(), !ctx.options.ignore_case) {
                    result.set_index(offset + local_idx);
                }
            }
        }
    }

    Ok(result)
}

/// Evaluate generic property access
fn eval_generic_property(
    ctx: &EvalContext,
    name: &str,
    op: CompareOp,
    value: &PropertyValue,
) -> EvalResult<SelectionResult> {
    // Map property names to atom fields
    match name.to_lowercase().as_str() {
        "b" | "b_factor" => {
            if let PropertyValue::Float(v) = value {
                eval_property(ctx, |atom, _| op.compare_f32(atom.b_factor, *v))
            } else {
                Err(EvalError::TypeMismatch {
                    expected: "float".to_string(),
                    got: format!("{:?}", value),
                })
            }
        }
        "q" | "occupancy" => {
            if let PropertyValue::Float(v) = value {
                eval_property(ctx, |atom, _| op.compare_f32(atom.occupancy, *v))
            } else {
                Err(EvalError::TypeMismatch {
                    expected: "float".to_string(),
                    got: format!("{:?}", value),
                })
            }
        }
        "name" => {
            if let PropertyValue::String(v) = value {
                let pattern = Pattern::Exact(v.clone());
                eval_property(ctx, |atom, _| {
                    pattern.matches(&atom.name, !ctx.options.ignore_case)
                })
            } else {
                Err(EvalError::TypeMismatch {
                    expected: "string".to_string(),
                    got: format!("{:?}", value),
                })
            }
        }
        _ => Err(EvalError::InvalidProperty(name.to_string())),
    }
}

/// Evaluate "present" - atoms with coordinates in current state
fn eval_present(ctx: &EvalContext) -> EvalResult<SelectionResult> {
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        let state = ctx.effective_state(mol);
        if let Some(cs) = mol.get_coord_set(state) {
            for local_idx in 0..mol.atom_count() {
                if cs.get_atom_coord(AtomIndex(local_idx as u32)).is_some() {
                    result.set_index(offset + local_idx);
                }
            }
        }
    }

    Ok(result)
}

/// Evaluate "around" - atoms within distance of selection (exclusive)
fn eval_around(ctx: &EvalContext, dist: f32, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());
    let dist_sq = dist * dist;

    // Build spatial grid from selected atoms
    let mut grid = SpatialGrid::with_capacity(dist, inner_result.count());
    let mut selected_coords: Vec<Vec3> = Vec::with_capacity(inner_result.count());

    for (mol, offset) in ctx.molecules_with_offsets() {
        let state = ctx.effective_state(mol);
        if let Some(cs) = mol.get_coord_set(state) {
            for local_idx in 0..mol.atom_count() {
                let global_idx = offset + local_idx;
                if inner_result.contains_index(global_idx) {
                    if let Some(coord) = cs.get_atom_coord(AtomIndex(local_idx as u32)) {
                        grid.insert(coord, selected_coords.len());
                        selected_coords.push(coord);
                    }
                }
            }
        }
    }

    // Query grid for each non-selected atom
    let mut neighbors = Vec::new();
    for (mol, offset) in ctx.molecules_with_offsets() {
        let state = ctx.effective_state(mol);
        if let Some(cs) = mol.get_coord_set(state) {
            for local_idx in 0..mol.atom_count() {
                let global_idx = offset + local_idx;
                // Skip atoms in the original selection
                if inner_result.contains_index(global_idx) {
                    continue;
                }
                if let Some(coord) = cs.get_atom_coord(AtomIndex(local_idx as u32)) {
                    grid.query_neighbors(coord, &mut neighbors);
                    for &ni in &neighbors {
                        let d = coord - selected_coords[ni];
                        if d.magnitude_squared() <= dist_sq {
                            result.set_index(global_idx);
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Evaluate "expand" - selection plus atoms within distance (inclusive)
fn eval_expand(ctx: &EvalContext, dist: f32, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let around = eval_around(ctx, dist, inner)?;
    Ok(inner_result.union(&around))
}

/// Evaluate "extend" - expand selection by N bonds
fn eval_extend(ctx: &EvalContext, n: u32, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let mut result = eval_expr(inner, ctx)?;

    for _ in 0..n {
        let mut new_atoms = SelectionResult::new(ctx.total_atoms());

        for (mol, offset) in ctx.molecules_with_offsets() {
            for (local_idx, _atom) in mol.atoms().enumerate() {
                let global_idx = offset + local_idx;
                if result.contains_index(global_idx) {
                    // Add all bonded atoms
                    for bonded in mol.bonded_atoms(AtomIndex(local_idx as u32)) {
                        new_atoms.set_index(offset + bonded.as_usize());
                    }
                }
            }
        }

        result.union_with(&new_atoms);
    }

    Ok(result)
}

/// Evaluate "gap" - atoms separated by gap from selection
fn eval_gap(ctx: &EvalContext, dist: f32, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    // Gap is similar to around but uses VdW surface distance
    // For simplicity, treat as around for now
    eval_around(ctx, dist, inner)
}

/// Evaluate "within" - atoms in source within distance of target
fn eval_within(
    ctx: &EvalContext,
    dist: f32,
    source: &SelectionExpr,
    target: &SelectionExpr,
) -> EvalResult<SelectionResult> {
    let source_result = eval_expr(source, ctx)?;
    let target_result = eval_expr(target, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());
    let dist_sq = dist * dist;

    // Build spatial grid from target coordinates
    let mut grid = SpatialGrid::with_capacity(dist, target_result.count());
    let mut target_coords: Vec<Vec3> = Vec::with_capacity(target_result.count());

    for (mol, offset) in ctx.molecules_with_offsets() {
        let state = ctx.effective_state(mol);
        if let Some(cs) = mol.get_coord_set(state) {
            for local_idx in 0..mol.atom_count() {
                let global_idx = offset + local_idx;
                if target_result.contains_index(global_idx) {
                    if let Some(coord) = cs.get_atom_coord(AtomIndex(local_idx as u32)) {
                        grid.insert(coord, target_coords.len());
                        target_coords.push(coord);
                    }
                }
            }
        }
    }

    // Query grid for each source atom
    let mut neighbors = Vec::new();
    for (mol, offset) in ctx.molecules_with_offsets() {
        let state = ctx.effective_state(mol);
        if let Some(cs) = mol.get_coord_set(state) {
            for local_idx in 0..mol.atom_count() {
                let global_idx = offset + local_idx;
                if !source_result.contains_index(global_idx) {
                    continue;
                }
                if let Some(coord) = cs.get_atom_coord(AtomIndex(local_idx as u32)) {
                    grid.query_neighbors(coord, &mut neighbors);
                    for &ni in &neighbors {
                        let d = coord - target_coords[ni];
                        if d.magnitude_squared() <= dist_sq {
                            result.set_index(global_idx);
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Evaluate "beyond" - atoms in source beyond distance from target
fn eval_beyond(
    ctx: &EvalContext,
    dist: f32,
    source: &SelectionExpr,
    target: &SelectionExpr,
) -> EvalResult<SelectionResult> {
    let source_result = eval_expr(source, ctx)?;
    let within_result = eval_within(ctx, dist, source, target)?;
    Ok(source_result.difference(&within_result))
}

/// Evaluate "near_to" - similar to within
fn eval_near_to(
    ctx: &EvalContext,
    dist: f32,
    source: &SelectionExpr,
    target: &SelectionExpr,
) -> EvalResult<SelectionResult> {
    eval_within(ctx, dist, source, target)
}

/// Evaluate "byres" - expand to complete residues
fn eval_byres(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        // Find residues with any selected atom
        let mut selected_residues: Vec<(String, i32, char, String)> = Vec::new();

        for (local_idx, atom) in mol.atoms().enumerate() {
            if inner_result.contains_index(offset + local_idx) {
                let key = (
                    atom.residue.chain.clone(),
                    atom.residue.resv,
                    atom.residue.inscode,
                    atom.residue.segi.clone(),
                );
                if !selected_residues.contains(&key) {
                    selected_residues.push(key);
                }
            }
        }

        // Select all atoms in those residues
        for (local_idx, atom) in mol.atoms().enumerate() {
            let key = (
                atom.residue.chain.clone(),
                atom.residue.resv,
                atom.residue.inscode,
                atom.residue.segi.clone(),
            );
            if selected_residues.contains(&key) {
                result.set_index(offset + local_idx);
            }
        }
    }

    Ok(result)
}

/// Evaluate "bychain" - expand to complete chains
fn eval_bychain(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        let mut selected_chains: Vec<(String, String)> = Vec::new();

        for (local_idx, atom) in mol.atoms().enumerate() {
            if inner_result.contains_index(offset + local_idx) {
                let key = (atom.residue.chain.clone(), atom.residue.segi.clone());
                if !selected_chains.contains(&key) {
                    selected_chains.push(key);
                }
            }
        }

        for (local_idx, atom) in mol.atoms().enumerate() {
            let key = (atom.residue.chain.clone(), atom.residue.segi.clone());
            if selected_chains.contains(&key) {
                result.set_index(offset + local_idx);
            }
        }
    }

    Ok(result)
}

/// Evaluate "byobject" - expand to complete objects/models
fn eval_byobject(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        // Check if any atom in this molecule is selected
        let has_selected =
            (0..mol.atom_count()).any(|local_idx| inner_result.contains_index(offset + local_idx));

        if has_selected {
            // Select all atoms in this molecule
            for local_idx in 0..mol.atom_count() {
                result.set_index(offset + local_idx);
            }
        }
    }

    Ok(result)
}

/// Evaluate "bymolecule" - expand to connected molecules
fn eval_bymolecule(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        // Find connected components (fragments)
        let mut visited = vec![false; mol.atom_count()];
        let mut component = vec![0usize; mol.atom_count()];
        let mut comp_id = 0;

        for start in 0..mol.atom_count() {
            if visited[start] {
                continue;
            }
            comp_id += 1;
            let mut stack = vec![start];
            while let Some(idx) = stack.pop() {
                if visited[idx] {
                    continue;
                }
                visited[idx] = true;
                component[idx] = comp_id;

                for bonded in mol.bonded_atoms(AtomIndex(idx as u32)) {
                    if !visited[bonded.as_usize()] {
                        stack.push(bonded.as_usize());
                    }
                }
            }
        }

        // Find components with selected atoms
        let mut selected_components = vec![false; comp_id + 1];
        for local_idx in 0..mol.atom_count() {
            if inner_result.contains_index(offset + local_idx) {
                selected_components[component[local_idx]] = true;
            }
        }

        // Select all atoms in those components
        for local_idx in 0..mol.atom_count() {
            if selected_components[component[local_idx]] {
                result.set_index(offset + local_idx);
            }
        }
    }

    Ok(result)
}

/// Evaluate "bysegment" - expand to complete segments
fn eval_bysegment(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        let mut selected_segis: Vec<String> = Vec::new();

        for (local_idx, atom) in mol.atoms().enumerate() {
            if inner_result.contains_index(offset + local_idx)
                && !selected_segis.contains(&atom.residue.segi)
            {
                selected_segis.push(atom.residue.segi.clone());
            }
        }

        for (local_idx, atom) in mol.atoms().enumerate() {
            if selected_segis.contains(&atom.residue.segi) {
                result.set_index(offset + local_idx);
            }
        }
    }

    Ok(result)
}

/// Evaluate "byfragment" - same as bymolecule
fn eval_byfragment(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    eval_bymolecule(ctx, inner)
}

/// Evaluate "bycalpha" - select C-alpha atoms of residues
fn eval_bycalpha(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let byres_result = eval_byres(ctx, inner)?;
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        for (local_idx, atom) in mol.atoms().enumerate() {
            if byres_result.contains_index(offset + local_idx) && atom.is_ca() {
                result.set_index(offset + local_idx);
            }
        }
    }

    Ok(result)
}

/// Evaluate "byring" - select atoms in rings
fn eval_byring(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    // Ring detection requires graph cycle analysis
    // For now, just return the input selection
    eval_expr(inner, ctx)
}

/// Evaluate "bycell" - select by unit cell
fn eval_bycell(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    // Unit cell selection requires crystallographic symmetry
    // For now, just return the input selection
    eval_expr(inner, ctx)
}

/// Evaluate "neighbor" - atoms bonded to selection (exclusive)
fn eval_neighbor(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());

    for (mol, offset) in ctx.molecules_with_offsets() {
        for (local_idx, _atom) in mol.atoms().enumerate() {
            if inner_result.contains_index(offset + local_idx) {
                for bonded in mol.bonded_atoms(AtomIndex(local_idx as u32)) {
                    let bonded_global = offset + bonded.as_usize();
                    // Exclude atoms in original selection
                    if !inner_result.contains_index(bonded_global) {
                        result.set_index(bonded_global);
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Evaluate "bound_to" - atoms bonded to selection (inclusive)
fn eval_bound_to(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let neighbor_result = eval_neighbor(ctx, inner)?;
    Ok(inner_result.union(&neighbor_result))
}

/// Evaluate "first" - first atom in selection
fn eval_first(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());

    if let Some(first_idx) = inner_result.first() {
        result.set(first_idx);
    }

    Ok(result)
}

/// Evaluate "last" - last atom in selection
fn eval_last(ctx: &EvalContext, inner: &SelectionExpr) -> EvalResult<SelectionResult> {
    let inner_result = eval_expr(inner, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());

    if let Some(last_idx) = inner_result.last() {
        result.set(last_idx);
    }

    Ok(result)
}

/// Evaluate "like" - residue-level pattern matching
fn eval_like(
    ctx: &EvalContext,
    left: &SelectionExpr,
    right: &SelectionExpr,
) -> EvalResult<SelectionResult> {
    // "like" matches residues with the same name
    let left_result = eval_expr(left, ctx)?;
    let right_result = eval_expr(right, ctx)?;
    let mut result = SelectionResult::new(ctx.total_atoms());

    // Get residue names from right selection
    let mut target_resns: Vec<String> = Vec::new();
    for (mol, offset) in ctx.molecules_with_offsets() {
        for (local_idx, atom) in mol.atoms().enumerate() {
            if right_result.contains_index(offset + local_idx)
                && !target_resns.contains(&atom.residue.resn)
            {
                target_resns.push(atom.residue.resn.clone());
            }
        }
    }

    // Select atoms from left with matching residue names
    for (mol, offset) in ctx.molecules_with_offsets() {
        for (local_idx, atom) in mol.atoms().enumerate() {
            if left_result.contains_index(offset + local_idx)
                && target_resns.contains(&atom.residue.resn)
            {
                result.set_index(offset + local_idx);
            }
        }
    }

    Ok(result)
}

/// Evaluate "in" - membership test
fn eval_in(
    ctx: &EvalContext,
    left: &SelectionExpr,
    right: &SelectionExpr,
) -> EvalResult<SelectionResult> {
    let left_result = eval_expr(left, ctx)?;
    let right_result = eval_expr(right, ctx)?;
    Ok(left_result.intersection(&right_result))
}

/// Evaluate macro (slash) notation
fn eval_macro(ctx: &EvalContext, spec: &MacroSpec) -> EvalResult<SelectionResult> {
    eval_property(ctx, |atom, info| {
        // Check each component
        if let Some(ref pattern) = spec.model {
            if let Some(mol) = ctx.molecule(info.mol_idx) {
                if !pattern.matches(&mol.name, !ctx.options.ignore_case) {
                    return false;
                }
            }
        }
        if let Some(ref pattern) = spec.segi {
            if !pattern.matches(&atom.residue.segi, !ctx.options.ignore_case_chain) {
                return false;
            }
        }
        if let Some(ref pattern) = spec.chain {
            if !pattern.matches(&atom.residue.chain, !ctx.options.ignore_case_chain) {
                return false;
            }
        }
        if let Some(ref pattern) = spec.resn {
            if !pattern.matches(&atom.residue.resn, !ctx.options.ignore_case) {
                return false;
            }
        }
        if let Some(ref resi_spec) = spec.resi {
            if !resi_spec.matches(atom.residue.resv, atom.residue.inscode) {
                return false;
            }
        }
        if let Some(ref pattern) = spec.name {
            if !pattern.matches(&atom.name, !ctx.options.ignore_case) {
                return false;
            }
        }
        if let Some(ref pattern) = spec.alt {
            if !pattern.matches(&atom.alt.to_string(), !ctx.options.ignore_case) {
                return false;
            }
        }
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::ResiSpec;
    use patinae_mol::{Atom, AtomResidue, BondOrder, CoordSet, Element, ObjectMolecule};
    use std::sync::Arc;

    fn create_test_molecule() -> ObjectMolecule {
        let mut mol = ObjectMolecule::new("test");

        // Create shared residues
        let ala_res = Arc::new(AtomResidue::from_parts("A", "ALA", 1, ' ', ""));
        let gly_res = Arc::new(AtomResidue::from_parts("A", "GLY", 2, ' ', ""));

        // Create a simple peptide-like structure
        let atoms = vec![
            ("N", ala_res.clone(), Element::Nitrogen),
            ("CA", ala_res.clone(), Element::Carbon),
            ("C", ala_res.clone(), Element::Carbon),
            ("O", ala_res.clone(), Element::Oxygen),
            ("N", gly_res.clone(), Element::Nitrogen),
            ("CA", gly_res.clone(), Element::Carbon),
            ("C", gly_res.clone(), Element::Carbon),
            ("O", gly_res.clone(), Element::Oxygen),
            ("H", gly_res.clone(), Element::Hydrogen),
        ];

        for (name, residue, elem) in atoms {
            let mut atom = Atom::new(name, elem);
            atom.residue = residue;
            atom.state.flags = AtomFlags::PROTEIN;
            mol.add_atom(atom);
        }

        // Add some bonds
        mol.add_bond(AtomIndex(0), AtomIndex(1), BondOrder::Single)
            .ok();
        mol.add_bond(AtomIndex(1), AtomIndex(2), BondOrder::Single)
            .ok();
        mol.add_bond(AtomIndex(2), AtomIndex(3), BondOrder::Double)
            .ok();
        mol.add_bond(AtomIndex(2), AtomIndex(4), BondOrder::Single)
            .ok();
        mol.add_bond(AtomIndex(4), AtomIndex(5), BondOrder::Single)
            .ok();
        mol.add_bond(AtomIndex(5), AtomIndex(6), BondOrder::Single)
            .ok();
        mol.add_bond(AtomIndex(6), AtomIndex(7), BondOrder::Double)
            .ok();
        mol.add_bond(AtomIndex(5), AtomIndex(8), BondOrder::Single)
            .ok();

        // Add coordinates
        let coords = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.5, 0.0, 0.0),
            Vec3::new(2.5, 1.0, 0.0),
            Vec3::new(2.5, 2.0, 0.0),
            Vec3::new(3.5, 0.5, 0.0),
            Vec3::new(4.5, 1.5, 0.0),
            Vec3::new(5.5, 1.0, 0.0),
            Vec3::new(6.5, 1.5, 0.0),
            Vec3::new(4.5, 2.5, 0.0),
        ];
        mol.add_coord_set(CoordSet::from_vec3(&coords));

        mol
    }

    #[test]
    fn test_eval_all() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let result = evaluate(&SelectionExpr::All, &ctx).unwrap();
        assert_eq!(result.count(), 9);
    }

    #[test]
    fn test_eval_none() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let result = evaluate(&SelectionExpr::None, &ctx).unwrap();
        assert_eq!(result.count(), 0);
    }

    #[test]
    fn test_eval_name() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let expr = SelectionExpr::Name(Pattern::Exact("CA".to_string()));
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 2); // Two CA atoms
    }

    #[test]
    fn label_predicate_uses_context_labels_not_atom_render_state() {
        let mut mol = create_test_molecule();
        mol.get_atom_mut(AtomIndex(0)).unwrap().repr.label = "legacy".to_string();
        let mut ctx = EvalContext::single(&mol);
        ctx.add_atom_label(1, "semantic");

        let semantic = SelectionExpr::Label(Pattern::Exact("semantic".to_string()));
        let semantic_result = evaluate(&semantic, &ctx).unwrap();
        assert_eq!(
            semantic_result.indices().collect::<Vec<_>>(),
            vec![AtomIndex(1)]
        );

        let legacy = SelectionExpr::Label(Pattern::Exact("legacy".to_string()));
        assert_eq!(evaluate(&legacy, &ctx).unwrap().count(), 0);
    }

    #[test]
    fn test_eval_resn() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let expr = SelectionExpr::Resn(Pattern::Exact("ALA".to_string()));
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 4); // ALA has 4 atoms
    }

    #[test]
    fn test_eval_resi() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let expr = SelectionExpr::Resi(ResiSpec::single(2));
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 5); // GLY has 5 atoms
    }

    #[test]
    fn test_eval_and() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let expr = SelectionExpr::And(
            Box::new(SelectionExpr::Name(Pattern::Exact("CA".to_string()))),
            Box::new(SelectionExpr::Resn(Pattern::Exact("ALA".to_string()))),
        );
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 1); // Only one CA in ALA
    }

    #[test]
    fn test_eval_or() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let expr = SelectionExpr::Or(
            Box::new(SelectionExpr::Name(Pattern::Exact("CA".to_string()))),
            Box::new(SelectionExpr::Name(Pattern::Exact("N".to_string()))),
        );
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 4); // Two CA and two N
    }

    #[test]
    fn test_eval_not() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let expr = SelectionExpr::Not(Box::new(SelectionExpr::Hydrogens));
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 8); // All except 1 hydrogen
    }

    #[test]
    fn test_eval_hydrogens() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let result = evaluate(&SelectionExpr::Hydrogens, &ctx).unwrap();
        assert_eq!(result.count(), 1);
    }

    #[test]
    fn test_eval_byres() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        // Select CA of ALA, expand to full residue
        let expr = SelectionExpr::ByRes(Box::new(SelectionExpr::And(
            Box::new(SelectionExpr::Name(Pattern::Exact("CA".to_string()))),
            Box::new(SelectionExpr::Resn(Pattern::Exact("ALA".to_string()))),
        )));
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 4); // Full ALA residue
    }

    #[test]
    fn test_eval_neighbor() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        // Neighbors of CA in ALA
        let expr = SelectionExpr::Neighbor(Box::new(SelectionExpr::And(
            Box::new(SelectionExpr::Name(Pattern::Exact("CA".to_string()))),
            Box::new(SelectionExpr::Resn(Pattern::Exact("ALA".to_string()))),
        )));
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 2); // N and C bonded to CA
    }

    #[test]
    fn test_eval_first_last() {
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);

        let first_expr = SelectionExpr::First(Box::new(SelectionExpr::All));
        let first_result = evaluate(&first_expr, &ctx).unwrap();
        assert_eq!(first_result.count(), 1);
        assert!(first_result.contains(AtomIndex(0)));

        let last_expr = SelectionExpr::Last(Box::new(SelectionExpr::All));
        let last_result = evaluate(&last_expr, &ctx).unwrap();
        assert_eq!(last_result.count(), 1);
        assert!(last_result.contains(AtomIndex(8)));
    }

    #[test]
    fn test_unknown_name_in_compound_expr() {
        let mol = create_test_molecule();
        let mut ctx = EvalContext::single(&mol);
        ctx.add_selection("obj1".to_string(), SelectionResult::all(mol.atom_count()));

        let expr = crate::parse("chain A or blabla").unwrap();
        assert!(evaluate(&expr, &ctx).is_err());
    }

    #[test]
    fn test_unknown_name_standalone() {
        let mol = create_test_molecule();
        let mut ctx = EvalContext::single(&mol);
        ctx.add_selection("obj1".to_string(), SelectionResult::all(mol.atom_count()));

        let expr = crate::parse("blabla").unwrap();
        assert!(evaluate(&expr, &ctx).is_err());
    }

    #[test]
    fn test_eval_macro_leading_slash() {
        // /test///1/CA → model=test, resi=1, name=CA → 1 atom (CA in ALA resi 1)
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let expr = crate::parse("/test///1/CA").unwrap();
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 1);
        assert!(result.contains(AtomIndex(1))); // CA in ALA (resi 1)
    }

    #[test]
    fn test_eval_macro_no_leading_slash() {
        // A/1/CA → chain=A, resi=1, name=CA (right-to-left)
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let expr = crate::parse("A/1/CA").unwrap();
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 1);
        assert!(result.contains(AtomIndex(1))); // CA in chain A, resi 1
    }

    #[test]
    fn test_eval_macro_resn() {
        // /test//A/ALA/ → model=test, chain=A, resn=ALA → all ALA atoms in chain A
        let mol = create_test_molecule();
        let ctx = EvalContext::single(&mol);
        let expr = crate::parse("/test//A/ALA/").unwrap();
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 4); // All 4 atoms in ALA residue
    }

    /// Create a molecule with two chains differing only by case: "A" and "a"
    fn create_mixed_case_chain_molecule() -> ObjectMolecule {
        let mut mol = ObjectMolecule::new("test");

        let chain_upper = Arc::new(AtomResidue::from_parts("A", "ALA", 1, ' ', ""));
        let chain_lower = Arc::new(AtomResidue::from_parts("a", "GLY", 1, ' ', ""));

        for (name, residue, elem) in [
            ("N", chain_upper.clone(), Element::Nitrogen),
            ("CA", chain_upper.clone(), Element::Carbon),
            ("N", chain_lower.clone(), Element::Nitrogen),
            ("CA", chain_lower.clone(), Element::Carbon),
        ] {
            let mut atom = Atom::new(name, elem);
            atom.residue = residue;
            mol.add_atom(atom);
        }

        mol
    }

    fn create_blank_and_named_chain_molecule() -> ObjectMolecule {
        let mut mol = ObjectMolecule::new("test");

        let blank_chain = Arc::new(AtomResidue::from_parts("", "THR", 4, ' ', ""));
        let named_chain = Arc::new(AtomResidue::from_parts("A", "THR", 4, ' ', ""));

        for (name, residue, elem) in [
            ("N", blank_chain.clone(), Element::Nitrogen),
            ("CA", blank_chain.clone(), Element::Carbon),
            ("N", named_chain.clone(), Element::Nitrogen),
            ("CA", named_chain.clone(), Element::Carbon),
        ] {
            let mut atom = Atom::new(name, elem);
            atom.residue = residue;
            mol.add_atom(atom);
        }

        mol
    }

    #[test]
    fn test_chain_empty_string_selects_only_blank_chain() {
        let mol = create_blank_and_named_chain_molecule();
        let ctx = EvalContext::single(&mol);

        let expr = crate::parse("chain \"\" and resi 4").unwrap();
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 2);
        assert!(result.contains(AtomIndex(0)));
        assert!(result.contains(AtomIndex(1)));
        assert!(!result.contains(AtomIndex(2)));
        assert!(!result.contains(AtomIndex(3)));
    }

    #[test]
    fn test_chain_selection_case_sensitive_by_default() {
        let mol = create_mixed_case_chain_molecule();
        let ctx = EvalContext::single(&mol); // defaults: ignore_case_chain = false

        // "chain A" should only match chain "A", not "a"
        let expr = SelectionExpr::Chain(Pattern::Exact("A".to_string()));
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 2); // Only the 2 atoms in chain "A"

        // "chain a" should only match chain "a", not "A"
        let expr = SelectionExpr::Chain(Pattern::Exact("a".to_string()));
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 2); // Only the 2 atoms in chain "a"
    }

    #[test]
    fn test_chain_selection_case_insensitive_when_setting_enabled() {
        let mol = create_mixed_case_chain_molecule();
        let mut ctx = EvalContext::single(&mol);
        ctx.options.ignore_case_chain = true; // override: make chain matching case-insensitive

        // "chain A" should now match both "A" and "a"
        let expr = SelectionExpr::Chain(Pattern::Exact("A".to_string()));
        let result = evaluate(&expr, &ctx).unwrap();
        assert_eq!(result.count(), 4); // All 4 atoms
    }
}
