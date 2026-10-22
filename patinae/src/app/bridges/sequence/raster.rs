//! Raster for the Sequence viewer.
use super::*;
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
pub(super) struct GlyphAtlas {
    font: FontRef<'static>,
    md_scale: f32,
    xs_scale: f32,
    md: HashMap<char, Glyph>,
    xs: HashMap<char, Glyph>,
}

impl GlyphAtlas {
    pub(super) fn new(dpr: f32, metrics: &SequenceMetrics) -> Option<Self> {
        let font = FontRef::try_from_slice(FONT_BYTES).ok()?;
        let md_scale = metrics.font * dpr;
        let xs_scale = metrics.small_font * dpr;
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

impl SequenceBridge {
    pub(super) fn push_to_slint(
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

        let strip_h_phys = (self.metrics.height * dpr).ceil() as u32;
        let mut top = 0.0;

        let mut tmp_slots: Vec<ChainSlot> = Vec::new();

        for seq_obj in &self.model.sequences {
            for (chain_idx_in_obj, chain) in seq_obj.chains.iter().enumerate() {
                if chain_idx_in_obj == 0 {
                    top += ss.get_object_header_height();
                }
                let mut cell_widths: Vec<f32> = Vec::with_capacity(chain.residues.len());
                let mut cell_prefix: Vec<f32> = Vec::with_capacity(chain.residues.len() + 1);
                cell_prefix.push(0.0);
                let mut sum = 0.0_f32;

                for r in &chain.residues {
                    let w =
                        (r.display_label.len() as f32 * self.metrics.cell).max(self.metrics.cell);
                    cell_widths.push(w);
                    sum += w;
                    cell_prefix.push(sum);
                }

                let strip_w_css = self.metrics.offset + sum;
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
                    top,
                });
                top += self.metrics.height + ss.get_row_gap();

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

        self.slots = tmp_slots;
        self.rendered_rows = 0..0;
        ss.set_chains(ModelRc::from(Rc::new(VecModel::from(chains))));
        ss.set_has_data(!self.model.sequences.is_empty());
        ss.set_max_row_width((max_row_w + self.metrics.padding).max(1.0));
        ss.set_footer_text("drag to add \u{00b7} \u{2318}+drag to exclude".into());
        ss.set_viewer_colors(self.viewer_colors);
        self.refresh_visible_rows(window, dpr, dark_mode);
    }

    /// Keep raster images only for visible rows and one row of overscan.
    pub(super) fn refresh_visible_rows(&mut self, window: &AppWindow, dpr: f32, dark_mode: bool) {
        let ss = window.global::<SequenceState>();
        let range = visible_rows(
            &self.slots,
            -ss.get_viewport_y(),
            ss.get_viewport_height(),
            self.metrics.height,
        );
        if range == self.rendered_rows {
            return;
        }
        let timing = std::env::var_os("PATINAE_INPUT_TIMING").map(|_| std::time::Instant::now());
        let rows = ss.get_chains();
        for index in self.rendered_rows.clone() {
            if !range.contains(&index) {
                if let Some(mut row) = rows.row_data(index) {
                    row.strip_image = Image::default();
                    row.highlight_image = Image::default();
                    rows.set_row_data(index, row);
                }
            }
        }
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
        let highlight = if dark_mode {
            [244, 114, 182, 77]
        } else {
            [190, 24, 93, 77]
        };
        // The atlas is mutable during rasterization; keep the immutable slots
        // outside that borrow without cloning residue strings on every scroll.
        let slots = std::mem::take(&mut self.slots);
        for index in range.clone() {
            if !self.rendered_rows.contains(&index) {
                if let Some(mut row) = rows.row_data(index) {
                    row.strip_image = self.render_strip_image(&slots[index], dpr, ghost, ligand);
                    row.highlight_image =
                        self.render_highlight_image(&slots[index], dpr, highlight);
                    rows.set_row_data(index, row);
                }
            }
        }
        self.slots = slots;
        self.rendered_rows = range;
        if let Some(started) = timing {
            eprintln!(
                "[patinae-sequence] rows={}..{} raster_ms={:.3}",
                self.rendered_rows.start,
                self.rendered_rows.end,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// Render the static strip image (ruler ticks + colored residue
    /// letters) for one chain.
    pub(super) fn render_strip_image(
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

        let letter_baseline_y = self.metrics.letter_baseline * dpr;
        let ruler_baseline_y = self.metrics.ruler_baseline * dpr;

        // ── Chain label (left edge of the strip) ─────────────────────
        let label_x0 = 2.0 * dpr;
        let label_x1 = self.metrics.label * dpr;
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
        let res_x_off = self.metrics.offset * dpr;
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
    pub(super) fn update_highlights(&mut self, kernel: &AppKernel, window: &AppWindow) {
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

        // Rebuild highlights only for rows that currently have raster images.
        let dpr = self.last_dpr.max(1.0);
        let dark_mode = self.last_dark_mode.unwrap_or(true);
        let hl_rgba = if dark_mode {
            // Theme.magenta dark = #F472B6, 0.30 alpha
            [244, 114, 182, 77]
        } else {
            // Theme.magenta light = #BE185D, 0.30 alpha
            [190, 24, 93, 77]
        };

        let ss = window.global::<SequenceState>();
        let chains_model = ss.get_chains();
        for ci in self.rendered_rows.clone() {
            let Some(mut row) = chains_model.row_data(ci) else {
                continue;
            };
            row.highlight_image = self.render_highlight_image(&self.slots[ci], dpr, hl_rgba);
            chains_model.set_row_data(ci, row);
        }
    }

    pub(super) fn render_highlight_image(
        &self,
        slot: &ChainSlot,
        dpr: f32,
        rgba: [u8; 4],
    ) -> Image {
        let w = slot.strip_w_phys;
        let h = slot.strip_h_phys;
        if w == 0 || h == 0 || self.model.highlighted.is_empty() {
            return Image::default();
        }
        let mut buf = vec![0u8; (w * h * 4) as usize];

        // Highlight band sits in the letter row (10 CSS px → STRIP_H CSS px).
        let y0 = (self.metrics.ruler * dpr) as u32;
        let y1 = h;

        let highlights = &self.model.highlighted;
        // Walk residues, painting magenta bands for hits.
        let res_x_off = self.metrics.offset * dpr;
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
}
pub(super) fn visible_rows(
    slots: &[ChainSlot],
    top: f32,
    height: f32,
    row_height: f32,
) -> std::ops::Range<usize> {
    if height <= 0.0 || !height.is_finite() || !top.is_finite() {
        return 0..0;
    }
    let start = slots.partition_point(|slot| slot.top + row_height < top - row_height);
    let end = slots.partition_point(|slot| slot.top <= top + height + row_height);
    start..end
}

pub(super) fn rgba_to_slint_image(buf: Vec<u8>, w: u32, h: u32) -> Image {
    let pixel_buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&buf, w, h);
    Image::from_rgba8(pixel_buf)
}

pub(super) fn fill_rect(
    buf: &mut [u8],
    stride_w: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    rgba: [u8; 4],
) {
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

#[expect(
    clippy::too_many_arguments,
    reason = "local raster helper keeps text target geometry explicit"
)]
pub(super) fn draw_text_centered(
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

#[expect(
    clippy::too_many_arguments,
    reason = "local raster helper keeps text target geometry explicit"
)]
pub(super) fn draw_text_left(
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

pub(super) fn resolve_chain_rgb(chain_id: &str, palette: &ThemedPalette) -> [u8; 3] {
    let color = palette.chains.get(chain_id);
    [
        (color.r * 255.0).clamp(0.0, 255.0) as u8,
        (color.g * 255.0).clamp(0.0, 255.0) as u8,
        (color.b * 255.0).clamp(0.0, 255.0) as u8,
    ]
}
