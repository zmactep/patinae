//! Geometry for the Sequence viewer.
use super::*;

impl SequenceBridge {
    /// Map a residue-strip-relative x coordinate to a residue index.
    ///
    /// `x_px` is relative to the residue strip's TouchArea (which starts
    /// exactly at the first residue cell). Returns `(idx, true)` for valid
    /// hits, `(0, false)` when the chain is empty.
    pub(super) fn x_to_residue(&self, chain_idx: i32, x_px: f32) -> (i32, bool) {
        let ci = chain_idx as usize;
        let slot = match self.slots.get(ci) {
            Some(s) => s,
            None => return (0, false),
        };
        if slot.cell_widths.is_empty() {
            return (0, false);
        }
        if x_px < 0.0 {
            return (0, true);
        }
        // Binary search over the cumulative prefix table.
        // `cell_prefix[i]` is the left edge of cell `i`.
        let n = slot.cell_widths.len();
        let prefix = &slot.cell_prefix;
        let last = prefix[n]; // right edge of last cell
        if x_px >= last {
            return ((n - 1) as i32, true);
        }
        let mut lo = 0usize;
        let mut hi = n;
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if prefix[mid] <= x_px {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo as i32, true)
    }

    /// Strict hover hit testing; drag intentionally clamps to the closest cell.
    pub(super) fn hit_residue(&self, chain_idx: i32, x_px: f32) -> Option<(i32, f32, f32)> {
        let slot = self.slots.get(usize::try_from(chain_idx).ok()?)?;
        let end = *slot.cell_prefix.last()?;
        if !x_px.is_finite() || x_px < 0.0 || x_px >= end {
            return None;
        }
        let (index, valid) = self.x_to_residue(chain_idx, x_px);
        valid.then(|| {
            (
                index,
                slot.cell_prefix[index as usize],
                slot.cell_prefix[index as usize + 1],
            )
        })
    }

    /// Resolve `(chain_index, residue_index)` to a tooltip string and the
    /// residue's identifying triple. Returns `None` when out of bounds.
    pub(super) fn resolve_residue(
        &self,
        chain_idx: i32,
        res_idx: i32,
    ) -> Option<(String, ResidueRef)> {
        let slot = self.slots.get(chain_idx as usize)?;
        let r = slot.chain.residues.get(res_idx as usize)?;
        let tooltip = format!("{}.{} {}", slot.chain.chain_id, r.resn, r.resv);
        let id = ResidueRef {
            object_name: slot.object_name.clone(),
            chain_id: slot.chain.chain_id.clone(),
            resv: r.resv,
        };
        Some((tooltip, id))
    }

    pub(super) fn chain_fasta(&self, chain_idx: i32) -> Option<(String, usize, String)> {
        let slot = self.slots.get(chain_idx as usize)?;
        let (text, count) = chain_to_fasta(&slot.object_name, &slot.chain)?;
        Some((text, count, slot.chain.chain_id.clone()))
    }

    pub(super) fn all_fasta(&self) -> Option<(String, usize)> {
        let (text, count) = collect_all_sequences(&self.model.sequences);
        if text.is_empty() {
            None
        } else {
            Some((text, count))
        }
    }

    /// Build a `select sele, ...` command from drag endpoints.
    ///
    /// Returns `None` if the chain/residue indices are out of bounds, or if
    /// `exclude` is set and `sele` does not exist (nothing to exclude from).
    pub(super) fn build_drag_command(
        &self,
        chain_idx: i32,
        start_idx: i32,
        end_idx: i32,
        exclude: bool,
        has_sele: bool,
    ) -> Option<String> {
        let slot = self.slots.get(chain_idx as usize)?;
        let n = slot.chain.residues.len();
        let si = start_idx.max(0) as usize;
        let ei = end_idx.max(0) as usize;
        if si >= n || ei >= n {
            return None;
        }
        let (lo, hi) = (si.min(ei), si.max(ei));
        let resv_values: Vec<i32> = slot.chain.residues[lo..=hi]
            .iter()
            .map(|r| r.resv)
            .collect();
        let object_name = format_exact_selector_value(&slot.object_name);
        let chain_id = format_exact_selector_value(&slot.chain.chain_id);
        let expr = format!(
            "model {object_name} and chain {chain_id} and resi {}",
            compress_resi_list(&resv_values)
        );
        build_sele_command(&expr, exclude, has_sele)
    }

    /// Update `SequenceState.drag-px-start / drag-px-end` from the current
    /// drag endpoint indices so the Slint preview rectangle matches the
    /// covered residues exactly.
    pub(super) fn write_drag_band(&self, window: &AppWindow, chain_idx: i32, start: i32, end: i32) {
        let Some(slot) = self.slots.get(chain_idx as usize) else {
            return;
        };
        let n = slot.cell_widths.len();
        if n == 0 {
            return;
        }
        let lo = start.max(0).min((n - 1) as i32) as usize;
        let hi = end.max(0).min((n - 1) as i32) as usize;
        let (a, b) = (lo.min(hi), lo.max(hi));
        // Strip-relative pixel band — includes the label/spacer offset so
        // the Slint preview Rectangle (positioned inside the strip Rect)
        // lands exactly on the residue cells.
        let x0 = self.metrics.offset + slot.cell_prefix[a];
        let x1 = self.metrics.offset + slot.cell_prefix[b + 1];
        let ss = window.global::<SequenceState>();
        ss.set_drag_px_start(x0);
        ss.set_drag_px_end(x1);
    }
}
