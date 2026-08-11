//! Contextual PDB chain identifiers.

use std::collections::HashMap;

// Rust guideline compliant 2026-02-21

#[derive(Debug)]
struct SourceSegments {
    ordinal: usize,
    effective: String,
    closed: bool,
}

impl SourceSegments {
    fn new(source: &str) -> Self {
        Self {
            ordinal: 1,
            effective: effective_chain_id(source, 1),
            closed: false,
        }
    }

    fn reopen(&mut self, source: &str) {
        self.ordinal = self
            .ordinal
            .checked_add(1)
            .expect("PDB chain segment ordinal must fit in usize");
        self.effective = effective_chain_id(source, self.ordinal);
        self.closed = false;
    }
}

/// Tracks contextual chain identifiers within one PDB model.
#[derive(Debug, Default)]
pub(super) struct ChainIdTracker {
    sources: HashMap<String, SourceSegments>,
    active_source: Option<String>,
}

impl ChainIdTracker {
    /// Resolve one successfully parsed atom's effective chain identifier.
    pub(super) fn effective_chain(&mut self, source: &str) -> String {
        if self.active_source.as_deref() != Some(source) {
            self.close_active();

            let segments = self
                .sources
                .entry(source.to_owned())
                .or_insert_with(|| SourceSegments::new(source));
            if segments.closed {
                segments.reopen(source);
            }
            self.active_source = Some(source.to_owned());
        }

        self.sources
            .get(source)
            .expect("active PDB source chain must have segment state")
            .effective
            .clone()
    }

    /// Close the segment containing the most recent valid atom.
    pub(super) fn terminate_active(&mut self) {
        self.close_active();
    }

    /// Clear all contextual identifiers at a PDB model boundary.
    pub(super) fn reset(&mut self) {
        self.sources.clear();
        self.active_source = None;
    }

    fn close_active(&mut self) {
        let Some(source) = self.active_source.take() else {
            return;
        };
        if let Some(segments) = self.sources.get_mut(&source) {
            segments.closed = true;
        }
    }
}

/// Decode a contextual identifier to its one-character PDB source chain.
pub(super) fn source_chain_id(effective: &str) -> &str {
    if let Some(ordinal) = effective.strip_prefix("__") {
        if is_canonical_ordinal(ordinal) {
            return "_";
        }
    }
    if let Some(ordinal) = effective.strip_prefix('_') {
        if is_canonical_ordinal(ordinal) {
            return "";
        }
    }

    let source_len = effective.chars().next().map_or(0, char::len_utf8);
    let ordinal = &effective[source_len..];
    if source_len > 0 && is_canonical_ordinal(ordinal) {
        &effective[..source_len]
    } else {
        effective
    }
}

fn effective_chain_id(source: &str, ordinal: usize) -> String {
    if ordinal == 1 {
        return source.to_owned();
    }

    match source {
        "" => format!("_{ordinal}"),
        "_" => format!("__{ordinal}"),
        _ => format!("{source}{ordinal}"),
    }
}

fn is_canonical_ordinal(value: &str) -> bool {
    let Some(first) = value.as_bytes().first() else {
        return false;
    };
    if !first.is_ascii_digit() || *first == b'0' {
        return false;
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }

    value.parse::<usize>().is_ok_and(|ordinal| ordinal >= 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contextual_chain_ids_decode_to_source_chains() {
        for (effective, source) in [
            ("", ""),
            ("_2", ""),
            ("_3", ""),
            ("_", "_"),
            ("__2", "_"),
            ("__3", "_"),
            ("A", "A"),
            ("A2", "A"),
            ("A19", "A"),
        ] {
            assert_eq!(source_chain_id(effective), source);
        }
    }

    #[test]
    fn noncanonical_suffixes_are_not_decoded() {
        for chain in ["_0", "_1", "_02", "__1", "A0", "A1", "A02", "AB2"] {
            assert_eq!(source_chain_id(chain), chain);
        }
    }
}
