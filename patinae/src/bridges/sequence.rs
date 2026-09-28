use std::cell::RefCell;
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

use crate::{AppWindow, SeqChainRow, SequenceState, Theme as ThemeGlobal};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Approximate pixel width of a single monospace character at the strip
/// font size (JetBrains Mono). Drives multi-char ligand label cells and
/// drag x → residue index mapping.
const CHAR_PX: f32 = 7.5;
/// Minimum cell width for single-letter residues. Matches `CHAR_PX` so
/// letters sit nearly edge-to-edge, like classic sequence viewers.
const MIN_CELL_PX: f32 = 7.5;
/// Chain-label zone (CSS px) at the start of every strip image.
const LABEL_AREA_PX: f32 = 24.0;
/// Gap between the chain-label glyphs and the first residue cell.
const LABEL_SPACER_PX: f32 = 4.0;
/// CSS x offset of the first residue cell within the strip image.
const RESIDUE_X_OFFSET: f32 = LABEL_AREA_PX + LABEL_SPACER_PX;
/// Padding added to max-row-width so the last cell isn't flush against the
/// scroll edge.
const MAX_ROW_PADDING_PX: f32 = 8.0;
/// Strip height in CSS pixels: 10 px ruler + 18 px letter row.
const STRIP_H_CSS: f32 = 28.0;
const RULER_H_CSS: f32 = 10.0;
/// Font sizes — strip font exceeds the compact UI label size for readability.
const FONT_MD_PX: f32 = 12.0;
const FONT_XS_PX: f32 = 9.0;

/// Bundled font (also imported by Foundations typography for the Slint side).
const FONT_BYTES: &[u8] = include_bytes!("../../ui/fonts/JetBrainsMono-Regular.ttf");

// ---------------------------------------------------------------------------
// Glyph atlas
// ---------------------------------------------------------------------------

/// Pre-rasterised alpha mask for a single glyph.
struct Glyph {
    width: u32,
    height: u32,
    /// Offset from the pen origin (baseline + nominal x) to the top-left
    /// of the bitmap, in physical pixels.
    bearing_x: i32,
    bearing_y: i32,
    /// Horizontal advance in physical pixels.
    advance: f32,
    /// Alpha coverage row-major.
    pixels: Vec<u8>,
}

/// Cached glyph maps for the two font sizes we use, scaled to a given DPR.
struct GlyphAtlas {
    font: FontRef<'static>,
    md_scale: f32,
    xs_scale: f32,
    md: HashMap<char, Glyph>,
    xs: HashMap<char, Glyph>,
}

impl GlyphAtlas {
    fn new(dpr: f32) -> Option<Self> {
        let font = FontRef::try_from_slice(FONT_BYTES).ok()?;
        let md_scale = FONT_MD_PX * dpr;
        let xs_scale = FONT_XS_PX * dpr;
        let mut atlas = Self {
            font,
            md_scale,
            xs_scale,
            md: HashMap::new(),
            xs: HashMap::new(),
        };
        // Warm the atlas with characters we'll always need.
        let warm = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789[]?-_*";
        for c in warm.chars() {
            atlas.glyph_md(c);
            atlas.glyph_xs(c);
        }
        Some(atlas)
    }

    fn glyph_md(&mut self, c: char) -> Option<&Glyph> {
        let scale = self.md_scale;
        if !self.md.contains_key(&c) {
            let glyph = rasterise(&self.font, scale, c)?;
            self.md.insert(c, glyph);
        }
        self.md.get(&c)
    }

    fn glyph_xs(&mut self, c: char) -> Option<&Glyph> {
        let scale = self.xs_scale;
        if !self.xs.contains_key(&c) {
            let glyph = rasterise(&self.font, scale, c)?;
            self.xs.insert(c, glyph);
        }
        self.xs.get(&c)
    }
}

fn rasterise(font: &FontRef<'static>, px_scale: f32, c: char) -> Option<Glyph> {
    let scale = PxScale::from(px_scale);
    let scaled = font.as_scaled(scale);
    let glyph_id = font.glyph_id(c);
    let advance = scaled.h_advance(glyph_id);
    let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));

    if let Some(outlined) = font.outline_glyph(glyph) {
        let bb = outlined.px_bounds();
        let w = bb.width().ceil() as u32;
        let h = bb.height().ceil() as u32;
        let mut pixels = vec![0u8; (w * h) as usize];
        outlined.draw(|x, y, coverage| {
            if x < w && y < h {
                let alpha = (coverage.clamp(0.0, 1.0) * 255.0) as u8;
                pixels[(y * w + x) as usize] = alpha;
            }
        });
        Some(Glyph {
            width: w,
            height: h,
            bearing_x: bb.min.x.floor() as i32,
            bearing_y: bb.min.y.floor() as i32,
            advance,
            pixels,
        })
    } else {
        // Whitespace / no outline — record advance only.
        Some(Glyph {
            width: 0,
            height: 0,
            bearing_x: 0,
            bearing_y: 0,
            advance,
            pixels: Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// SequenceBridge
// ---------------------------------------------------------------------------

/// Per-row data captured during `push_to_slint` so callbacks (drag, hover)
/// can resolve a chain index back to its source object/chain without
/// re-walking the model.
struct ChainSlot {
    object_name: String,
    chain: SeqChain,
    /// Per-residue cell widths in CSS pixels (drives drag x → index mapping
    /// and strip image layout).
    cell_widths: Vec<f32>,
    /// Cumulative CSS-pixel offsets relative to the first residue cell
    /// (length = residues + 1). Add `RESIDUE_X_OFFSET` to get strip-image
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
}

pub struct SequenceBridge {
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
    last_dark_mode: Option<bool>,
}

impl SequenceBridge {
    pub fn new() -> Self {
        Self {
            model: SequenceModel::new(),
            viewer_colors: false,
            cache_dirty: true,
            slots: Vec::new(),
            last_sele_generation: None,
            atlas: None,
            last_dpr: 0.0,
            last_dark_mode: None,
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

        // DPR or theme change → regenerate atlas / strip images.
        if (dpr - self.last_dpr).abs() > 0.001 {
            self.atlas = GlyphAtlas::new(dpr);
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
            let color_ctx = SequenceColorContext {
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

    /// Map a residue-strip-relative x coordinate to a residue index.
    ///
    /// `x_px` is relative to the residue strip's TouchArea (which starts
    /// exactly at the first residue cell). Returns `(idx, true)` for valid
    /// hits, `(0, false)` when the chain is empty.
    pub fn x_to_residue(&self, chain_idx: i32, x_px: f32) -> (i32, bool) {
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

    /// Resolve `(chain_index, residue_index)` to a tooltip string and the
    /// residue's identifying triple. Returns `None` when out of bounds.
    pub fn resolve_residue(&self, chain_idx: i32, res_idx: i32) -> Option<(String, ResidueRef)> {
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

    pub fn chain_fasta(&self, chain_idx: i32) -> Option<(String, usize, String)> {
        let slot = self.slots.get(chain_idx as usize)?;
        let (text, count) = chain_to_fasta(&slot.object_name, &slot.chain)?;
        Some((text, count, slot.chain.chain_id.clone()))
    }

    pub fn all_fasta(&self) -> Option<(String, usize)> {
        let (text, count) = collect_all_sequences(&self.model.sequences);
        if text.is_empty() {
            None
        } else {
            Some((text, count))
        }
    }

    // --- Internal helpers ---

    fn push_to_slint(
        &mut self,
        window: &AppWindow,
        palette: &ThemedPalette,
        dpr: f32,
        dark_mode: bool,
    ) {
        let ss = window.global::<SequenceState>();

        let mut chains: Vec<SeqChainRow> = Vec::new();
        self.slots.clear();
        let mut max_row_w: f32 = 0.0;

        let ghost = if dark_mode {
            [58, 58, 67]
        } else {
            [154, 148, 136]
        };
        let ligand = if dark_mode {
            [244, 114, 182]
        } else {
            [190, 24, 93]
        };
        let strip_h_phys = (STRIP_H_CSS * dpr).ceil() as u32;

        let mut tmp_slots: Vec<ChainSlot> = Vec::new();

        for seq_obj in &self.model.sequences {
            for (chain_idx_in_obj, chain) in seq_obj.chains.iter().enumerate() {
                let mut cell_widths: Vec<f32> = Vec::with_capacity(chain.residues.len());
                let mut cell_prefix: Vec<f32> = Vec::with_capacity(chain.residues.len() + 1);
                cell_prefix.push(0.0);
                let mut sum = 0.0_f32;

                for r in &chain.residues {
                    let w = (r.display_label.len() as f32 * CHAR_PX).max(MIN_CELL_PX);
                    cell_widths.push(w);
                    sum += w;
                    cell_prefix.push(sum);
                }

                let strip_w_css = RESIDUE_X_OFFSET + sum;
                let strip_w_phys = (strip_w_css * dpr).ceil().max(1.0) as u32;
                let chain_rgb = resolve_chain_rgb(&chain.chain_id, palette);

                tmp_slots.push(ChainSlot {
                    object_name: seq_obj.object_name.clone(),
                    chain: chain.clone(),
                    cell_widths,
                    cell_prefix,
                    strip_w_phys,
                    strip_h_phys,
                    chain_rgb,
                });

                let chain_color =
                    slint::Color::from_rgb_u8(chain_rgb[0], chain_rgb[1], chain_rgb[2]);
                if strip_w_css > max_row_w {
                    max_row_w = strip_w_css;
                }

                // Push placeholder; real image filled below once the
                // atlas mut-borrow doesn't conflict with slot iteration.
                chains.push(SeqChainRow {
                    chain_id: chain.chain_id.clone().into(),
                    chain_color,
                    object_name: seq_obj.object_name.clone().into(),
                    is_first_in_object: chain_idx_in_obj == 0,
                    polymer_length: chain
                        .residues
                        .iter()
                        .filter(|residue| residue.kind.is_polymer())
                        .count() as i32,
                    strip_image: Image::default(),
                    highlight_image: Image::default(),
                    strip_width_px: strip_w_css,
                });
            }
        }

        // Render strip + (empty) highlight image for each slot now that we
        // can take a mutable borrow of `self.atlas`.
        for (i, slot) in tmp_slots.iter().enumerate() {
            let strip_img = self.render_strip_image(slot, dpr, ghost, ligand);
            let highlight_img = make_empty_image(slot.strip_w_phys, slot.strip_h_phys);
            chains[i].strip_image = strip_img;
            chains[i].highlight_image = highlight_img;
        }

        self.slots = tmp_slots;
        ss.set_chains(ModelRc::from(Rc::new(VecModel::from(chains))));
        ss.set_has_data(!self.model.sequences.is_empty());
        ss.set_max_row_width((max_row_w + MAX_ROW_PADDING_PX).max(1.0));
        ss.set_footer_text("drag to add \u{00b7} \u{2318}+drag to exclude".into());
        ss.set_viewer_colors(self.viewer_colors);
    }

    /// Render the static strip image (ruler ticks + colored residue
    /// letters) for one chain.
    fn render_strip_image(
        &mut self,
        slot: &ChainSlot,
        dpr: f32,
        ghost_rgb: [u8; 3],
        ligand_rgb: [u8; 3],
    ) -> Image {
        let w = slot.strip_w_phys;
        let h = slot.strip_h_phys;
        let mut buf = vec![0u8; (w * h * 4) as usize];

        let atlas = match self.atlas.as_mut() {
            Some(a) => a,
            None => return Image::default(),
        };

        // Baselines are derived from a single anchor — centring 12-pt
        // glyphs in the 28-px row. The chain-label and residue letters
        // share this baseline because both are rasterised here.
        let letter_baseline_y = 20.0 * dpr;
        // Ticks live in the top 10 CSS px; smaller font, raised baseline.
        let ruler_baseline_y = 7.0 * dpr;

        // ── Chain label (left edge of the strip) ─────────────────────
        let label_x0 = 2.0 * dpr;
        let label_x1 = LABEL_AREA_PX * dpr;
        draw_text_left(
            &mut buf,
            w,
            h,
            label_x0,
            label_x1,
            letter_baseline_y,
            &slot.chain.chain_id,
            atlas,
            /* small= */ false,
            slot.chain_rgb,
        );

        // ── Residue cells ────────────────────────────────────────────
        let res_x_off = RESIDUE_X_OFFSET * dpr;
        for (i, r) in slot.chain.residues.iter().enumerate() {
            let cell_x0 = res_x_off + slot.cell_prefix[i] * dpr;
            let cell_x1 = res_x_off + slot.cell_prefix[i + 1] * dpr;

            // Letter row
            let letter_color = if matches!(r.kind, ResidueKind::Ligand) {
                ligand_rgb
            } else if self.viewer_colors {
                r.viewer_color
            } else {
                r.named_color
            };
            draw_text_centered(
                &mut buf,
                w,
                h,
                cell_x0,
                cell_x1,
                letter_baseline_y,
                &r.display_label,
                atlas,
                /* small= */ false,
                letter_color,
            );

            // Ruler tick (every 20 residues) — tick number centered above
            if r.resv % 20 == 0 {
                let label = format!("{}", r.resv);
                draw_text_centered(
                    &mut buf,
                    w,
                    h,
                    cell_x0,
                    cell_x1,
                    ruler_baseline_y,
                    &label,
                    atlas,
                    /* small= */ true,
                    ghost_rgb,
                );
            }
        }

        rgba_to_slint_image(buf, w, h)
    }

    /// Update per-residue `highlighted` flags from the 3D selection manager.
    fn update_highlights(&mut self, kernel: &AppKernel, window: &AppWindow) {
        // Fast-path: selection state hasn't structurally changed since
        // the last update — the highlight images are still current.
        let gen = kernel.session.selections.generation();
        if self.last_sele_generation == Some(gen) {
            return;
        }
        self.last_sele_generation = Some(gen);

        let mut new_highlights = std::collections::HashSet::new();

        if let Some(entry) = kernel.session.selections.get("sele") {
            for (obj_name, result) in &entry.cached_results {
                if let Some(mol_obj) = kernel.session.registry.get_molecule(obj_name) {
                    let mol = mol_obj.molecule();
                    let atoms = mol.atoms_slice();
                    for raw_idx in result.raw_indices() {
                        if let Some(atom) = atoms.get(raw_idx) {
                            new_highlights.insert(ResidueRef {
                                object_name: obj_name.clone(),
                                chain_id: atom.residue.chain.clone(),
                                resv: atom.residue.resv,
                            });
                        }
                    }
                }
            }
        }

        self.model.update_highlights(new_highlights);

        // Rebuild highlight image for each chain.
        let dpr = self.last_dpr.max(1.0);
        let dark_mode = self.last_dark_mode.unwrap_or(true);
        let hl_rgba = if dark_mode {
            // Theme.magenta dark = #F472B6, 0.30 alpha
            [244, 114, 182, 77]
        } else {
            // Theme.magenta light = #BE185D, 0.30 alpha
            [190, 24, 93, 77]
        };

        let mut new_images: Vec<Image> = Vec::with_capacity(self.slots.len());
        for slot in &self.slots {
            new_images.push(self.render_highlight_image(slot, dpr, hl_rgba));
        }

        let ss = window.global::<SequenceState>();
        let chains_model = ss.get_chains();
        let chain_count = chains_model.row_count().min(new_images.len());
        for (ci, image) in new_images.iter().enumerate().take(chain_count) {
            let Some(mut row) = chains_model.row_data(ci) else {
                continue;
            };
            row.highlight_image = image.clone();
            chains_model.set_row_data(ci, row);
        }
    }

    fn render_highlight_image(&self, slot: &ChainSlot, dpr: f32, rgba: [u8; 4]) -> Image {
        let w = slot.strip_w_phys;
        let h = slot.strip_h_phys;
        if w == 0 || h == 0 {
            return Image::default();
        }
        let mut buf = vec![0u8; (w * h * 4) as usize];

        // Highlight band sits in the letter row (10 CSS px → STRIP_H CSS px).
        let y0 = (RULER_H_CSS * dpr) as u32;
        let y1 = h;

        let highlights = &self.model.highlighted;
        if highlights.is_empty() {
            return rgba_to_slint_image(buf, w, h);
        }

        // Walk residues, painting magenta bands for hits.
        let res_x_off = RESIDUE_X_OFFSET * dpr;
        for (i, r) in slot.chain.residues.iter().enumerate() {
            let key = ResidueRef {
                object_name: slot.object_name.clone(),
                chain_id: slot.chain.chain_id.clone(),
                resv: r.resv,
            };
            if !highlights.contains(&key) {
                continue;
            }
            let x0 = (res_x_off + slot.cell_prefix[i] * dpr) as u32;
            let x1 = (res_x_off + slot.cell_prefix[i + 1] * dpr).ceil() as u32;
            let x1 = x1.min(w);
            fill_rect(&mut buf, w, x0, y0, x1, y1, rgba);
        }

        rgba_to_slint_image(buf, w, h)
    }

    // --- Drag-select ---

    /// Build a `select sele, ...` command from drag endpoints.
    ///
    /// Returns `None` if the chain/residue indices are out of bounds, or if
    /// `exclude` is set and `sele` does not exist (nothing to exclude from).
    fn build_drag_command(
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
    fn write_drag_band(&self, window: &AppWindow, chain_idx: i32, start: i32, end: i32) {
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
        let x0 = RESIDUE_X_OFFSET + slot.cell_prefix[a];
        let x1 = RESIDUE_X_OFFSET + slot.cell_prefix[b + 1];
        let ss = window.global::<SequenceState>();
        ss.set_drag_px_start(x0);
        ss.set_drag_px_end(x1);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_empty_image(w: u32, h: u32) -> Image {
    if w == 0 || h == 0 {
        return Image::default();
    }
    let buf = vec![0u8; (w * h * 4) as usize];
    rgba_to_slint_image(buf, w, h)
}

fn rgba_to_slint_image(buf: Vec<u8>, w: u32, h: u32) -> Image {
    let pixel_buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&buf, w, h);
    Image::from_rgba8(pixel_buf)
}

fn fill_rect(buf: &mut [u8], stride_w: u32, x0: u32, y0: u32, x1: u32, y1: u32, rgba: [u8; 4]) {
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    for y in y0..y1 {
        let row = (y * stride_w) as usize * 4;
        for x in x0..x1 {
            let idx = row + x as usize * 4;
            // Source-over blending against transparent background ≡ copy.
            buf[idx] = rgba[0];
            buf[idx + 1] = rgba[1];
            buf[idx + 2] = rgba[2];
            buf[idx + 3] = rgba[3];
        }
    }
}

/// Lay out a string within `[x0_phys, x1_phys]` and rasterise glyphs
/// centered horizontally on a baseline at `y_baseline_phys`.
#[expect(
    clippy::too_many_arguments,
    reason = "local raster helper keeps text target geometry explicit"
)]
fn draw_text_centered(
    buf: &mut [u8],
    stride_w: u32,
    stride_h: u32,
    x0_phys: f32,
    x1_phys: f32,
    y_baseline_phys: f32,
    text: &str,
    atlas: &mut GlyphAtlas,
    small: bool,
    color: [u8; 3],
) {
    // Compute total advance.
    let mut total = 0.0_f32;
    for c in text.chars() {
        let adv = if small {
            atlas.glyph_xs(c).map(|g| g.advance).unwrap_or(0.0)
        } else {
            atlas.glyph_md(c).map(|g| g.advance).unwrap_or(0.0)
        };
        total += adv;
    }
    let cell_w = x1_phys - x0_phys;
    let start_x = x0_phys + (cell_w - total) * 0.5;

    let mut pen_x = start_x;
    for c in text.chars() {
        let glyph_opt = if small {
            atlas.glyph_xs(c)
        } else {
            atlas.glyph_md(c)
        };
        let Some(glyph) = glyph_opt else { continue };
        if !glyph.pixels.is_empty() {
            blit_glyph(
                buf,
                stride_w,
                stride_h,
                glyph,
                pen_x,
                y_baseline_phys,
                color,
            );
        }
        pen_x += glyph.advance;
    }
}

/// Like `draw_text_centered` but left-aligned within `[x0_phys, x1_phys]`.
/// Used for the chain-id label at the start of the strip.
#[expect(
    clippy::too_many_arguments,
    reason = "local raster helper keeps text target geometry explicit"
)]
fn draw_text_left(
    buf: &mut [u8],
    stride_w: u32,
    stride_h: u32,
    x0_phys: f32,
    x1_phys: f32,
    y_baseline_phys: f32,
    text: &str,
    atlas: &mut GlyphAtlas,
    small: bool,
    color: [u8; 3],
) {
    let _ = x1_phys; // bounds-clip happens via stride_w in blit_glyph
    let mut pen_x = x0_phys;
    for c in text.chars() {
        let glyph_opt = if small {
            atlas.glyph_xs(c)
        } else {
            atlas.glyph_md(c)
        };
        let Some(glyph) = glyph_opt else { continue };
        if !glyph.pixels.is_empty() {
            blit_glyph(
                buf,
                stride_w,
                stride_h,
                glyph,
                pen_x,
                y_baseline_phys,
                color,
            );
        }
        pen_x += glyph.advance;
    }
}

fn blit_glyph(
    buf: &mut [u8],
    stride_w: u32,
    stride_h: u32,
    glyph: &Glyph,
    pen_x: f32,
    baseline_y: f32,
    color: [u8; 3],
) {
    let origin_x = pen_x.round() as i32 + glyph.bearing_x;
    let origin_y = baseline_y.round() as i32 + glyph.bearing_y;
    for gy in 0..glyph.height {
        let py = origin_y + gy as i32;
        if py < 0 || py as u32 >= stride_h {
            continue;
        }
        let row = (py as u32 * stride_w) as usize * 4;
        let grow = (gy * glyph.width) as usize;
        for gx in 0..glyph.width {
            let alpha = glyph.pixels[grow + gx as usize];
            if alpha == 0 {
                continue;
            }
            let px = origin_x + gx as i32;
            if px < 0 || px as u32 >= stride_w {
                continue;
            }
            let idx = row + px as usize * 4;
            // Source-over blending of (color, alpha) onto buf at idx.
            let sa = alpha as u32;
            let inv = 255 - sa;
            let dst_a = buf[idx + 3] as u32;
            let out_a = sa + (dst_a * inv + 127) / 255;
            // Premultiplied-style: dst ← src*sa + dst*(1-sa) approximately
            for k in 0..3 {
                let s = color[k] as u32 * sa;
                let d = buf[idx + k] as u32 * inv;
                buf[idx + k] = ((s + d + 127) / 255).min(255) as u8;
            }
            buf[idx + 3] = out_a.min(255) as u8;
        }
    }
}

// ---------------------------------------------------------------------------
// Callback wiring
// ---------------------------------------------------------------------------

pub fn setup_callbacks(app: Rc<RefCell<crate::app::App>>, window: &AppWindow) {
    let ss = window.global::<SequenceState>();

    // Residue clicked — placeholder, drag-ended subsumes single-click.
    ss.on_residue_clicked(move |_chain, _idx, _resv| {});

    // Row drag move — map x coordinate to residue index
    {
        let app = app.clone();
        let weak = window.as_weak();
        ss.on_row_drag_move(move |chain, x_px| {
            let a = app.borrow();
            let (idx, ok) = a.sequence.x_to_residue(chain, x_px);
            if !ok {
                return;
            }
            let Some(w) = weak.upgrade() else { return };
            let ss = w.global::<SequenceState>();
            let start = if ss.get_drag_start() < 0 {
                ss.set_drag_start(idx);
                idx
            } else {
                ss.get_drag_start()
            };
            ss.set_drag_end(idx);
            a.sequence.write_drag_band(&w, chain, start, idx);
        });
    }

    // Drag ended — build and execute select/deselect command via build_sele_command.
    {
        let app = app.clone();
        let weak = window.as_weak();
        ss.on_residue_drag_ended(move |chain, start_idx, _end_chain, end_idx| {
            // Guard against stray pointer-up with no real start.
            if start_idx < 0 || end_idx < 0 {
                return;
            }
            let Some(w) = weak.upgrade() else { return };
            let ss = w.global::<SequenceState>();
            let exclude = ss.get_ctrl_held();

            let mut a = app.borrow_mut();
            let has_sele = a.kernel.session.selections.contains("sele");
            let cmd = a
                .sequence
                .build_drag_command(chain, start_idx, end_idx, exclude, has_sele);
            if let Some(cmd) = cmd {
                a.kernel.bus.execute_command(cmd);
            }
        });
    }

    // Hover (no drag) — resolve residue under cursor and update tooltip state.
    {
        let app = app.clone();
        let weak = window.as_weak();
        ss.on_row_hover(move |chain, x_px| {
            let a = app.borrow();
            let (idx, ok) = a.sequence.x_to_residue(chain, x_px);
            let resolved = if ok {
                a.sequence.resolve_residue(chain, idx)
            } else {
                None
            };
            drop(a);
            let Some(w) = weak.upgrade() else { return };
            let ss = w.global::<SequenceState>();
            if let Some((tooltip, _id)) = resolved {
                ss.set_hover_chain(chain);
                ss.set_hover_residue(idx);
                ss.set_hover_tooltip(tooltip.into());
                ss.set_hover_active(true);
            } else {
                ss.set_hover_active(false);
            }
        });
    }

    // Viewer colors toggle
    {
        let app = app.clone();
        ss.on_viewer_colors_toggled(move || {
            let mut a = app.borrow_mut();
            a.sequence.toggle_viewer_colors();
            a.sequence.invalidate();
        });
    }

    {
        let app = app.clone();
        let weak = window.as_weak();
        ss.on_copy_chain_fasta(move |chain| {
            let Some(w) = weak.upgrade() else { return };
            let mut a = app.borrow_mut();
            match a.sequence.chain_fasta(chain) {
                Some((text, count, chain_id)) => {
                    match crate::clipboard::set_clipboard_text(&text) {
                        Ok(()) => {
                            let message = format!("Copied {count} residues from chain {chain_id}");
                            a.kernel.bus.print_info(message.clone());
                            a.notify_short(message);
                        }
                        Err(err) => {
                            let message = format!("Could not copy sequence to clipboard: {err}");
                            a.kernel.bus.print_warning(message.clone());
                            a.notify_short(message);
                        }
                    }
                    w.window().request_redraw();
                }
                None => {
                    let message = "No polymer sequence available for this chain.";
                    a.kernel.bus.print_warning(message);
                    a.notify_short(message);
                    w.window().request_redraw();
                }
            }
        });
    }

    {
        let app = app.clone();
        let weak = window.as_weak();
        ss.on_copy_all_fasta(move || {
            let Some(w) = weak.upgrade() else { return };
            let mut a = app.borrow_mut();
            match a.sequence.all_fasta() {
                Some((text, count)) => {
                    match crate::clipboard::set_clipboard_text(&text) {
                        Ok(()) => {
                            let message = format!("Copied {count} residues to clipboard");
                            a.kernel.bus.print_info(message.clone());
                            a.notify_short(message);
                        }
                        Err(err) => {
                            let message = format!("Could not copy sequence to clipboard: {err}");
                            a.kernel.bus.print_warning(message.clone());
                            a.notify_short(message);
                        }
                    }
                    w.window().request_redraw();
                }
                None => {
                    let message = "No polymer sequences available.";
                    a.kernel.bus.print_warning(message);
                    a.notify_short(message);
                    w.window().request_redraw();
                }
            }
        });
    }
}

fn resolve_chain_rgb(chain_id: &str, palette: &ThemedPalette) -> [u8; 3] {
    let color = palette.chains.get(chain_id);
    [
        (color.r * 255.0).clamp(0.0, 255.0) as u8,
        (color.g * 255.0).clamp(0.0, 255.0) as u8,
        (color.b * 255.0).clamp(0.0, 255.0) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_framework::model::sequence::SeqResidue;

    fn test_residue(resv: i32) -> SeqResidue {
        SeqResidue {
            chain: String::new(),
            resn: "THR".to_string(),
            resv,
            display_char: 'T',
            display_label: "T".to_string(),
            char_width: 1,
            atom_range: 0..1,
            named_color: [0, 0, 0],
            viewer_color: [0, 0, 0],
            kind: ResidueKind::AminoAcidCanonical,
        }
    }

    fn test_slot(chain_id: &str, residues: Vec<SeqResidue>) -> ChainSlot {
        ChainSlot {
            object_name: "test".to_string(),
            chain: SeqChain {
                chain_id: chain_id.to_string(),
                residues,
            },
            cell_widths: Vec::new(),
            cell_prefix: Vec::new(),
            strip_w_phys: 0,
            strip_h_phys: 0,
            chain_rgb: [0, 0, 0],
        }
    }

    #[test]
    fn viewer_color_toggle_and_invalidate_mark_cache_dirty() {
        let mut bridge = SequenceBridge::new();
        bridge.cache_dirty = false;

        bridge.toggle_viewer_colors();
        assert!(bridge.viewer_colors);
        assert!(bridge.cache_dirty);

        bridge.cache_dirty = false;
        bridge.invalidate();
        assert!(bridge.cache_dirty);
    }

    #[test]
    fn build_drag_command_quotes_blank_chain() {
        let mut bridge = SequenceBridge::new();
        bridge
            .slots
            .push(test_slot("", vec![test_residue(4), test_residue(5)]));

        let cmd = bridge.build_drag_command(0, 0, 1, false, false).unwrap();
        assert_eq!(cmd, "select sele, model test and chain \"\" and resi 4-5");
    }
}
