mod raster;
use raster::*;
mod geometry;
use std::collections::HashMap;
use std::rc::Rc;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel};

use patinae_color::ThemedPalette;
use patinae_framework::kernel::AppKernel;
use patinae_framework::model::sequence::{
    chain_to_fasta, collect_all_sequences, compress_resi_list, ResidueKind, ResidueRef, SeqChain,
    SequenceColorContext, SequenceModel,
};
use patinae_select::{build_sele_command, format_exact_selector_value};

use crate::{
    AppWindow, ComponentMetrics, SeqChainRow, SequenceMetrics, SequenceState, Theme as ThemeGlobal,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

// Geometry and font sizes are supplied by the shared Slint component metrics.

/// Bundled font (also imported by Foundations typography for the Slint side).
const FONT_BYTES: &[u8] = include_bytes!("../../../ui/fonts/JetBrainsMono-Regular.ttf");

// ---------------------------------------------------------------------------
// Glyph atlas
// ---------------------------------------------------------------------------

struct ChainSlot {
    object_name: String,
    chain: SeqChain,
    /// Per-residue cell widths in CSS pixels (drives drag x → index mapping
    /// and strip image layout).
    cell_widths: Vec<f32>,
    /// Cumulative CSS-pixel offsets relative to the first residue cell
    /// (length = residues + 1). Add `self.metrics.offset` to get strip-image
    /// coordinates.
    cell_prefix: Vec<f32>,
    /// Physical-pixel buffer width used to build the strip image (kept so
    /// the highlight overlay can be re-rendered at the same size without
    /// rebuilding the entire row).
    strip_w_phys: u32,
    /// Physical height (rounded once per sync).
    strip_h_phys: u32,
    /// Pre-resolved RGB for the chain-id label, baked into the strip image.
    chain_rgb: [u8; 3],
    /// Vertical position of the strip inside the scrollable content.
    top: f32,
}

pub struct SequenceBridge {
    gesture: events::Gesture,
    next_gesture: u64,
    pending_commits: std::collections::HashSet<u64>,
    generation: i32,
    model: SequenceModel,
    viewer_colors: bool,
    cache_dirty: bool,
    /// Flat list of chain rows in display order — same order as the Slint
    /// `chains` model. Indexed by `chain_idx` from Slint callbacks.
    slots: Vec<ChainSlot>,
    /// Last seen `SelectionManager.generation()`. Skip the per-residue
    /// highlight walk (which is O(chains × residues) for assemblies)
    /// when the selection hasn't structurally changed since.
    last_sele_generation: Option<u64>,
    /// Lazily built glyph atlas, regenerated when DPR changes.
    atlas: Option<GlyphAtlas>,
    last_dpr: f32,
    metrics: SequenceMetrics,
    last_dark_mode: Option<bool>,
    rendered_rows: std::ops::Range<usize>,
}

impl SequenceBridge {
    pub fn new() -> Self {
        Self {
            model: SequenceModel::new(),
            gesture: events::Gesture::Idle,
            next_gesture: 0,
            pending_commits: Default::default(),
            generation: 0,
            viewer_colors: false,
            cache_dirty: true,
            slots: Vec::new(),
            last_sele_generation: None,
            atlas: None,
            last_dpr: 0.0,
            metrics: SequenceMetrics::default(),
            last_dark_mode: None,
            rendered_rows: 0..0,
        }
    }

    /// Attach sequence models to Slint globals (call once after window creation).
    pub fn attach(&self, _window: &AppWindow) {
        // Models are pushed lazily on first sync().
    }

    /// Sync domain state → Slint models. Called each frame.
    pub fn sync(&mut self, kernel: &AppKernel, window: &AppWindow) {
        let registry = &kernel.session.registry;
        let dpr = window.window().scale_factor().max(1.0);
        let dark_mode = window.global::<ThemeGlobal>().get_dark_mode();

        let metrics = window.global::<ComponentMetrics>().get_sequence();

        // DPR or Foundations geometry change regenerates both glyphs and hit regions.
        if (dpr - self.last_dpr).abs() > 0.001 || self.metrics != metrics {
            self.atlas = GlyphAtlas::new(dpr, &metrics);
            self.metrics = metrics;
            self.last_dpr = dpr;
            self.cache_dirty = true;
        }
        if self.last_dark_mode != Some(dark_mode) {
            self.last_dark_mode = Some(dark_mode);
            self.cache_dirty = true;
        }

        let rebuild = self.cache_dirty || self.model.needs_rebuild(registry, self.viewer_colors);
        self.cache_dirty = false;

        if rebuild {
            self.generation = self.generation.wrapping_add(1);
            window
                .global::<SequenceState>()
                .set_model_generation(self.generation);
            window.global::<SequenceState>().invoke_clear_hover();
            let color_ctx = SequenceColorContext {
                settings: &kernel.session.settings,
                named_palette: &kernel.session.named_palette,
                palette: &kernel.session.palette,
            };
            self.model.rebuild_cache(registry, &color_ctx);
            // After a structural rebuild the previous highlight images
            // were drawn against a different geometry — force a redraw.
            self.last_sele_generation = None;
            self.push_to_slint(window, &kernel.session.palette, dpr, dark_mode);
        }

        // Update 3D selection highlights (no full rebuild needed)
        self.update_highlights(kernel, window);
        self.refresh_visible_rows(window, dpr, dark_mode);

        // Mirror viewer-colors flag for the toggle button.
        let ss = window.global::<SequenceState>();
        if ss.get_viewer_colors() != self.viewer_colors {
            ss.set_viewer_colors(self.viewer_colors);
        }
    }

    /// Invalidate cache — force rebuild on next sync.
    pub fn invalidate(&mut self) {
        self.cache_dirty = true;
    }

    /// Toggle viewer colors mode.
    pub fn toggle_viewer_colors(&mut self) {
        self.viewer_colors = !self.viewer_colors;
        self.cache_dirty = true;
    }

    // --- Public: drag coordinate → index mapping ---

    // --- Internal helpers ---

    // --- Drag-select ---
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) mod events;
pub(crate) use events::setup_callbacks;

#[cfg(test)]
mod tests;
