//! Selection language parser and evaluator.
//!
//! This crate provides a command selection language implementation,
//! allowing you to select atoms based on properties, spatial relationships,
//! and logical combinations.
//!
//! # Overview
//!
//! The selection language supports:
//! - Property selectors: `name`, `resn`, `resi`, `chain`, `elem`, etc.
//! - Numeric comparisons: `b > 50`, `q < 1`, `x > 0`
//! - Special selections: `all`, `none`, `hydrogens`, `backbone`, etc.
//! - Logical operators: `and`, `or`, `not`
//! - Distance operators: `around`, `within`, `beyond`, `expand`
//! - Grouping operators: `byres`, `bychain`, `bymolecule`, etc.
//! - Slash macro notation: `/model/segi/chain/resn`resi/name`
//!
//! # Example
//!
//! ```rust,ignore
//! use patinae_select::{select, SelectionResult};
//! use patinae_mol::ObjectMolecule;
//!
//! let mol: ObjectMolecule = /* load molecule */;
//!
//! // Select all C-alpha atoms in chain A
//! let result = select(&mol, "name CA and chain A")?;
//!
//! // Select atoms within 5 Angstroms of residue 100
//! let result = select(&mol, "all within 5 of resi 100")?;
//!
//! // Select entire residues containing any backbone atom near a ligand
//! let result = select(&mol, "byres (backbone around 4 of organic)")?;
//! ```

use std::borrow::Cow;

// Module declarations
mod ast;
mod context;
mod error;
mod eval;
mod keywords;
mod lexer;
mod parser;
mod pattern;
mod result;

// Re-export main types
pub use ast::{CompareOp, MacroSpec, SelectionExpr};
pub use context::{EvalContext, SelectionOptions};
pub use error::{EvalError, ParseError, SelectError, SelectResult};
pub use pattern::{IntSpec, Pattern, ResiItem, ResiSpec};
pub use result::SelectionResult;

// Re-export patinae-mol types for convenience
pub use patinae_mol::AtomIndex;

/// Parse a selection string into an AST
///
/// # Arguments
/// * `input` - The selection string to parse
///
/// # Returns
/// * `Ok(SelectionExpr)` - The parsed selection expression
/// * `Err(ParseError)` - If parsing fails
///
/// # Example
/// ```rust,ignore
/// let expr = parse("name CA and chain A")?;
/// ```
pub fn parse(input: &str) -> Result<SelectionExpr, ParseError> {
    parser::parse_selection(input)
}

/// Select atoms from a molecule using a selection string
///
/// This is the main entry point for most selection operations.
///
/// # Arguments
/// * `mol` - The molecule to select from
/// * `selection` - The selection string
///
/// # Returns
/// * `Ok(SelectionResult)` - A bitset indicating which atoms are selected
/// * `Err(SelectError)` - If parsing or evaluation fails
///
/// # Example
/// ```rust,ignore
/// let result = select(&mol, "name CA")?;
/// println!("Selected {} atoms", result.count());
/// ```
pub fn select(mol: &patinae_mol::ObjectMolecule, selection: &str) -> SelectResult<SelectionResult> {
    let expr = parse(selection)?;
    let ctx = EvalContext::single(mol);
    eval::evaluate(&expr, &ctx).map_err(SelectError::from)
}

/// Select atoms and return their indices
///
/// Convenience function that returns a Vec of AtomIndex instead of a bitset.
///
/// # Arguments
/// * `mol` - The molecule to select from
/// * `selection` - The selection string
///
/// # Returns
/// * `Ok(Vec<AtomIndex>)` - Indices of selected atoms
/// * `Err(SelectError)` - If parsing or evaluation fails
pub fn select_atoms(
    mol: &patinae_mol::ObjectMolecule,
    selection: &str,
) -> SelectResult<Vec<AtomIndex>> {
    let result = select(mol, selection)?;
    Ok(result.indices().collect())
}

/// Evaluate a pre-parsed selection expression with a context
///
/// Use this when you want to reuse a parsed expression or need
/// more control over the evaluation context (multiple molecules,
/// named selections, specific state).
///
/// # Arguments
/// * `expr` - The parsed selection expression
/// * `ctx` - The evaluation context
///
/// # Returns
/// * `Ok(SelectionResult)` - A bitset indicating which atoms are selected
/// * `Err(EvalError)` - If evaluation fails
pub fn evaluate(expr: &SelectionExpr, ctx: &EvalContext) -> Result<SelectionResult, EvalError> {
    eval::evaluate(expr, ctx)
}

/// Formats a string as an exact selector value.
///
/// Generated selection expressions use this for string-valued selectors such as
/// `model`, `chain`, `segi`, `resn`, and `name`. Values that can be represented
/// as a single exact unquoted pattern are left unchanged; empty or syntactically
/// special values are double-quoted and escaped.
///
/// # Examples
///
/// ```
/// use patinae_select::format_exact_selector_value;
///
/// assert_eq!(format_exact_selector_value("A").as_ref(), "A");
/// assert_eq!(format_exact_selector_value("").as_ref(), "\"\"");
/// ```
#[must_use]
pub fn format_exact_selector_value(value: &str) -> Cow<'_, str> {
    if is_unquoted_exact_selector_value(value) {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(quote_exact_selector_value(value))
    }
}

fn quote_exact_selector_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn is_unquoted_exact_selector_value(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }

    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    if first.is_alphabetic() || first == '_' {
        return chars.all(is_ident_char);
    }

    if first.is_ascii_digit() {
        return is_unquoted_digit_selector_value(value);
    }

    false
}

fn is_unquoted_digit_selector_value(value: &str) -> bool {
    let rest = value.trim_start_matches(|ch: char| ch.is_ascii_digit());
    if rest.is_empty() {
        return value.parse::<i32>().is_ok();
    }

    let mut chars = rest.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first.is_alphabetic() || first == '\'')
        && chars.all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '\'')
}

fn is_ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '.' || ch == '\''
}

/// Build a `select sele, ...` command that adds to, removes from,
/// or creates the `sele` selection.
///
/// Returns `None` when `exclude=true` but no `sele` exists yet (nothing to exclude from).
pub fn build_sele_command(expr: &str, exclude: bool, has_sele: bool) -> Option<String> {
    if exclude {
        has_sele.then(|| format!("select sele, sele and not ({expr})"))
    } else if has_sele {
        Some(format!("select sele, sele or ({expr})"))
    } else {
        Some(format!("select sele, {expr}"))
    }
}

/// Prelude module for convenient imports
pub mod prelude {
    pub use crate::ast::{CompareOp, SelectionExpr};
    pub use crate::context::EvalContext;
    pub use crate::error::{ParseError, SelectError, SelectResult};
    pub use crate::pattern::{Pattern, ResiSpec};
    pub use crate::result::SelectionResult;
    pub use crate::{evaluate, format_exact_selector_value, parse, select, select_atoms};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let expr = parse("all").unwrap();
        assert!(matches!(expr, SelectionExpr::All));
    }

    #[test]
    fn test_parse_name() {
        let expr = parse("name CA").unwrap();
        assert!(matches!(expr, SelectionExpr::Name(_)));
    }

    #[test]
    fn format_exact_selector_value_leaves_simple_values_unquoted() {
        assert_eq!(format_exact_selector_value("A").as_ref(), "A");
        assert_eq!(format_exact_selector_value("1abc").as_ref(), "1abc");
        assert_eq!(format_exact_selector_value("10").as_ref(), "10");
    }

    #[test]
    fn format_exact_selector_value_quotes_overflowing_integer() {
        let overflow = (i64::from(i32::MAX) + 1).to_string();
        let formatted = format_exact_selector_value(&overflow);

        assert_eq!(formatted.as_ref(), format!("\"{overflow}\""));
        assert!(matches!(
            parse(&format!("name {formatted}")),
            Ok(SelectionExpr::Name(Pattern::Exact(value))) if value == overflow
        ));
    }

    #[test]
    fn format_exact_selector_value_quotes_empty_and_special_values() {
        assert_eq!(format_exact_selector_value("").as_ref(), "\"\"");
        assert_eq!(
            format_exact_selector_value("chain A").as_ref(),
            "\"chain A\""
        );
        assert_eq!(format_exact_selector_value("A+B").as_ref(), "\"A+B\"");
        assert_eq!(
            format_exact_selector_value("A\"B\\C").as_ref(),
            "\"A\\\"B\\\\C\""
        );
    }
}
