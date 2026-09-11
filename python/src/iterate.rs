//! Helpers for `iterate` and `alter` commands.
//!
//! Provides functions to populate Python dicts with atom properties
//! and to apply modified properties back to atoms.

use std::sync::Arc;

use patinae_mol::{Atom, Element, SecondaryStructure};
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Populate a Python dict with atom properties for use as `locals` in
/// `py.run()` during iterate/alter.
pub fn set_atom_locals(
    locals: &Bound<'_, PyDict>,
    atom: &Atom,
    coord: Option<(f32, f32, f32)>,
    index: usize,
    model_name: &str,
) -> PyResult<()> {
    // Identity
    locals.set_item("name", &*atom.name)?;
    locals.set_item("resn", &atom.residue.resn)?;
    locals.set_item("resv", atom.residue.resv)?;
    locals.set_item("resi", atom.residue.resv)?;
    locals.set_item("chain", &atom.residue.chain)?;
    locals.set_item("segi", &atom.residue.segi)?;
    locals.set_item("alt", atom.alt.to_string())?;
    locals.set_item("elem", atom.element.symbol())?;

    // Physical
    locals.set_item("b", atom.b_factor)?;
    locals.set_item("q", atom.occupancy)?;
    locals.set_item("vdw", atom.effective_vdw())?;
    locals.set_item("partial_charge", atom.partial_charge)?;
    locals.set_item("formal_charge", atom.formal_charge)?;

    // State
    locals.set_item("ss", ss_to_str(atom.ss_type))?;
    locals.set_item("color", atom.repr.colors.base)?;
    locals.set_item("hetatm", atom.state.hetatm)?;
    locals.set_item("type", if atom.state.hetatm { "HETATM" } else { "ATOM" })?;

    // Index / ID
    locals.set_item("index", index + 1)?; // 1-based command API index.
    locals.set_item("ID", atom.id)?;
    locals.set_item("rank", atom.rank)?;

    // Context
    locals.set_item("model", model_name)?;

    // Coordinates
    if let Some((x, y, z)) = coord {
        locals.set_item("x", x)?;
        locals.set_item("y", y)?;
        locals.set_item("z", z)?;
    }

    Ok(())
}

/// Read mutable properties back from the Python locals dict and apply
/// changes to the atom. Only modifies fields that differ from the original.
pub fn apply_locals_to_atom(locals: &Bound<'_, PyDict>, atom: &mut Atom) -> PyResult<()> {
    // name
    if let Some(val) = locals.get_item("name")? {
        let new_name: String = val.extract()?;
        if &*atom.name != new_name.as_str() {
            atom.name = Arc::from(new_name);
        }
    }

    // b-factor
    if let Some(val) = locals.get_item("b")? {
        let v: f32 = val.extract()?;
        atom.b_factor = v;
    }

    // occupancy
    if let Some(val) = locals.get_item("q")? {
        let v: f32 = val.extract()?;
        atom.occupancy = v;
    }

    // vdw
    if let Some(val) = locals.get_item("vdw")? {
        let v: f32 = val.extract()?;
        atom.vdw = v;
    }

    // partial_charge
    if let Some(val) = locals.get_item("partial_charge")? {
        let v: f32 = val.extract()?;
        atom.partial_charge = v;
    }

    // formal_charge
    if let Some(val) = locals.get_item("formal_charge")? {
        let v: i8 = val.extract()?;
        atom.formal_charge = v;
    }

    // color
    if let Some(val) = locals.get_item("color")? {
        let v: i32 = val.extract()?;
        atom.repr.colors.base = v;
    }

    // element
    if let Some(val) = locals.get_item("elem")? {
        let sym: String = val.extract()?;
        if sym != atom.element.symbol() {
            if let Some(el) = Element::from_symbol(&sym) {
                atom.element = el;
            }
        }
    }

    // secondary structure
    if let Some(val) = locals.get_item("ss")? {
        let s: String = val.extract()?;
        let new_ss = str_to_ss(&s);
        if new_ss != atom.ss_type {
            atom.ss_type = new_ss;
        }
    }

    // type (ATOM/HETATM)
    if let Some(val) = locals.get_item("type")? {
        let t: String = val.extract()?;
        atom.state.hetatm = t == "HETATM";
    }

    // alt
    if let Some(val) = locals.get_item("alt")? {
        let a: String = val.extract()?;
        let c = a.chars().next().unwrap_or(' ');
        if c != atom.alt {
            atom.alt = c;
        }
    }

    // Residue fields (shared via Arc — clone-on-write)
    let mut residue_changed = false;
    let mut new_res = (*atom.residue).clone();

    if let Some(val) = locals.get_item("chain")? {
        let v: String = val.extract()?;
        if v != atom.residue.chain {
            new_res.key.chain = v;
            residue_changed = true;
        }
    }
    if let Some(val) = locals.get_item("resn")? {
        let v: String = val.extract()?;
        if v != atom.residue.resn {
            new_res.key.resn = v;
            residue_changed = true;
        }
    }
    if let Some(val) = locals.get_item("resv")? {
        let v: i32 = val.extract()?;
        if v != atom.residue.resv {
            new_res.key.resv = v;
            residue_changed = true;
        }
    }
    if let Some(val) = locals.get_item("segi")? {
        let v: String = val.extract()?;
        if v != atom.residue.segi {
            new_res.segi = v;
            residue_changed = true;
        }
    }

    if residue_changed {
        atom.residue = Arc::new(new_res);
    }

    Ok(())
}

/// Build the globals dict for `iterate`/`alter` expression evaluation.
///
/// Always uses `__main__.__dict__` as the base so that `stored`, `cmd`,
/// and any user-defined variables are visible to expressions.
/// When a `space` dict is provided, its entries are merged on top.
pub fn build_globals<'py>(
    py: Python<'py>,
    space: Option<&Bound<'py, PyDict>>,
) -> PyResult<Bound<'py, PyDict>> {
    let globals = py.import("__main__")?.dict();
    if let Some(s) = space {
        globals.update(s.as_mapping())?;
    }
    Ok(globals)
}

/// Convert a `SecondaryStructure` enum to the command API single-letter code.
pub fn ss_to_str(ss: SecondaryStructure) -> &'static str {
    match ss {
        SecondaryStructure::Helix | SecondaryStructure::Helix310 | SecondaryStructure::HelixPi => {
            "H"
        }
        SecondaryStructure::Sheet => "S",
        _ => "L",
    }
}

/// Convert a command API SS string to a `SecondaryStructure` enum.
pub fn str_to_ss(s: &str) -> SecondaryStructure {
    match s {
        "H" => SecondaryStructure::Helix,
        "S" => SecondaryStructure::Sheet,
        _ => SecondaryStructure::Loop,
    }
}
