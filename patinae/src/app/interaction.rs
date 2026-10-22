//! Interaction for the native application.

use super::*;

impl App {
    /// Clear hover indicators and reset hover state.
    pub(super) fn clear_hover_indicators(&mut self) {
        self.kernel.session.clear_hover();
        self.kernel.viewport.hover_hit = None;
        self.interaction.last_hover_mode = None;
    }

    /// Process hover indicators: ray-cast each frame when no button is pressed.
    pub(super) fn process_hover(&mut self, vw: u32, vh: u32, sf: f32, scene_origin: (f32, f32)) {
        if self.kernel.viewport.input.any_button_pressed() {
            return;
        }
        if self.rendering.renderer.is_none() {
            return;
        }
        if !self.interaction.cursor_known.get() {
            return;
        }

        // Cursor position from winit (physical pixels, window-relative).
        // Convert to viewport-relative by subtracting sidebar.
        let (wx, wy) = self.interaction.cursor_pos.get();
        let mx = wx - scene_origin.0 * sf;
        let my = wy - scene_origin.1 * sf;
        if mx < 0.0 || mx > vw as f32 || my < 0.0 || my > vh as f32 {
            // Cursor is outside the viewport (over sidebar, dock, etc.)
            if self.kernel.viewport.hover_hit.is_some() {
                self.clear_hover_indicators();
            }
            return;
        }

        // Async hover pick: returns `None` when no fresh result is ready
        // this frame — keep the existing hover indicator until the next pick
        // resolves. `Some(_)` carries a fresh hit/miss to apply.
        let resolved = self
            .rendering
            .renderer
            .as_mut()
            .and_then(|r| r.poll_hover_pick(&self.kernel.session, mx as u32, my as u32));
        if let Some(new_hit) = resolved {
            self.apply_hover_result(new_hit);
        } else {
            self.refresh_cached_hover_mode();
        }
    }

    pub(super) fn effective_hover_mode(&self) -> MouseSelectionMode {
        if self.kernel.viewport.input.ctrl_or_cmd_held() {
            MouseSelectionMode::Atoms
        } else {
            self.kernel.session.settings.ui.mouse_selection_mode
        }
    }

    pub(super) fn apply_hover_result(&mut self, new_hit: Option<PickHit>) {
        let mode = self.effective_hover_mode();
        let hit_changed = match (&self.kernel.viewport.hover_hit, &new_hit) {
            (None, None) => false,
            (Some(_), None) | (None, Some(_)) => true,
            (Some(a), Some(b)) => a.object_name != b.object_name || a.atom_index != b.atom_index,
        };
        if !hit_changed && self.interaction.last_hover_mode == Some(mode) {
            return;
        }

        self.set_hover_from_hit(new_hit.as_ref(), mode);
        self.kernel.viewport.hover_hit = new_hit;
        self.interaction.last_hover_mode = Some(mode);
    }

    pub(super) fn refresh_cached_hover_mode(&mut self) {
        let mode = self.effective_hover_mode();
        if self.interaction.last_hover_mode == Some(mode) {
            return;
        }

        let hit = self.kernel.viewport.hover_hit.clone();
        self.set_hover_from_hit(hit.as_ref(), mode);
        self.interaction.last_hover_mode = Some(mode);
    }

    pub(super) fn set_hover_from_hit(&mut self, hit: Option<&PickHit>, mode: MouseSelectionMode) {
        let target = hit.and_then(|hit| {
            let molecule = self
                .kernel
                .session
                .registry
                .get_molecule(&hit.object_name)?;
            Some(patinae_scene::HoverTarget {
                object: hit.object_name.clone(),
                selection: expand_pick_to_selection(hit, mode.into(), molecule.molecule()),
            })
        });
        if let Some(target) = target {
            self.kernel.session.set_hover(target);
        } else {
            self.kernel.session.clear_hover();
        }
    }

    /// Process a click and report whether ordinary hover should be reset.
    pub(super) fn process_click(&mut self, click: PendingClick, viewport: (u32, u32)) -> bool {
        let clear_hover = !click.ctrl_or_cmd;
        let click_pos = click.position;
        // Ignore clicks outside the 3D viewport. The popover-dismiss backdrop
        // (layout.slint) forwards pointer-down/up to ViewportState so a drag
        // through the backdrop rotates the camera; a plain dismiss-click
        // would otherwise be seen here as a viewport miss and run
        // `deselect sele`, wiping the user's selection.
        let (vw, vh) = viewport;
        if click_pos.0 < 0.0
            || click_pos.0 >= vw as f32
            || click_pos.1 < 0.0
            || click_pos.1 >= vh as f32
        {
            return clear_hover;
        }

        if self.dismiss_viewport_image() {
            return false;
        }

        let hit = self.rendering.renderer.as_mut().and_then(|r| {
            r.pick_at_click(&self.kernel.session, click_pos.0 as u32, click_pos.1 as u32)
        });

        self.apply_pick_result(hit.as_ref(), click.ctrl_or_cmd);
        clear_hover
    }

    pub(super) fn apply_pick_result(&mut self, hit: Option<&PickHit>, ctrl_or_cmd: bool) {
        if ctrl_or_cmd {
            if let Some(hit) = hit {
                self.pick_recent_atom(hit);
            }
            return;
        }

        let cmd = if let Some(hit) = hit {
            if let Some(mol_obj) = self.kernel.session.registry.get_molecule(&hit.object_name) {
                let mol = mol_obj.molecule();
                let mode = self.kernel.session.settings.ui.mouse_selection_mode as i32;

                pick_expression_for_hit(hit, mode, mol).and_then(|expr| {
                    let overlaps_sele =
                        self.kernel
                            .session
                            .selections
                            .get("sele")
                            .is_some_and(|entry| {
                                let sel = expand_pick_to_selection(hit, mode, mol);
                                patinae_select::select(mol, &entry.expression)
                                    .map(|sele| sel.intersection(&sele).any())
                                    .unwrap_or(false)
                            });
                    let has_sele = self.kernel.session.selections.contains("sele");
                    build_sele_command(&expr, overlaps_sele, has_sele)
                })
            } else {
                None
            }
        } else {
            // Miss — clear selection if one exists.
            if self.kernel.session.selections.contains("sele") {
                Some("deselect sele".to_string())
            } else {
                None
            }
        };

        if let Some(cmd) = cmd {
            self.kernel.bus.execute_command(cmd);
        }
    }

    pub(super) fn pick_recent_atom(&mut self, hit: &PickHit) {
        let Some(molecule) = self.kernel.session.registry.get_molecule(&hit.object_name) else {
            self.kernel.bus.print_warning(
                "Recent atom pick was rejected: picked object is not a loaded molecule",
            );
            return;
        };
        let path = match canonical_atom_path_for_hit(hit, molecule.molecule()) {
            Ok(path) => path,
            Err(error) => {
                self.kernel
                    .bus
                    .print_warning(format!("Recent atom pick was rejected: {error}"));
                return;
            }
        };
        let command = if self.kernel.session.recent_atoms.row_id(&path).is_some() {
            "unpick"
        } else {
            "pick"
        };
        self.kernel
            .bus
            .execute_command(format!("{command} {}", quote_command_arg(&path)));
    }

    pub(super) fn dismiss_viewport_image(&mut self) -> bool {
        let had_cpu_image = self.kernel.session.viewport_image.take().is_some();
        let had_gpu_image = self
            .rendering
            .renderer
            .as_mut()
            .is_some_and(|renderer| renderer.clear_viewport_gpu_image());
        let had_image = had_cpu_image || had_gpu_image;
        if had_image {
            self.clear_hover_indicators();
        }
        had_image
    }

    pub(super) fn viewport_size(app: &AppWindow) -> (u32, u32) {
        viewport_size_from_parts(
            app.get_scene_width(),
            app.get_scene_height(),
            app.window().scale_factor(),
        )
    }
}

pub(super) fn viewport_size_from_parts(
    width_lp: f32,
    height_lp: f32,
    scale_factor: f32,
) -> (u32, u32) {
    let scale_factor = scale_factor.max(0.0);
    (
        (width_lp * scale_factor).max(1.0) as u32,
        (height_lp * scale_factor).max(1.0) as u32,
    )
}

pub(super) fn normalized_slint_shortcut_text(
    key: patinae_scene::KeyCode,
    shift: bool,
    ctrl: bool,
    alt: bool,
    super_key: bool,
) -> Option<&'static str> {
    if alt || !platform_shortcut_modifier_active(ctrl, super_key) {
        return None;
    }

    key.canonical_shortcut_text(shift)
}

#[cfg(target_os = "macos")]
pub(super) fn platform_shortcut_modifier_active(_ctrl: bool, super_key: bool) -> bool {
    super_key
}

#[cfg(not(target_os = "macos"))]
pub(super) fn platform_shortcut_modifier_active(ctrl: bool, _super_key: bool) -> bool {
    ctrl
}

pub(super) fn map_winit_key(
    key: slint::winit_030::winit::keyboard::KeyCode,
) -> Option<patinae_scene::KeyCode> {
    use patinae_scene::KeyCode as Pk;
    use slint::winit_030::winit::keyboard::KeyCode as Wk;

    Some(match key {
        Wk::Digit0 => Pk::Digit0,
        Wk::Digit1 => Pk::Digit1,
        Wk::Digit2 => Pk::Digit2,
        Wk::Digit3 => Pk::Digit3,
        Wk::Digit4 => Pk::Digit4,
        Wk::Digit5 => Pk::Digit5,
        Wk::Digit6 => Pk::Digit6,
        Wk::Digit7 => Pk::Digit7,
        Wk::Digit8 => Pk::Digit8,
        Wk::Digit9 => Pk::Digit9,
        Wk::KeyA => Pk::KeyA,
        Wk::KeyB => Pk::KeyB,
        Wk::KeyC => Pk::KeyC,
        Wk::KeyD => Pk::KeyD,
        Wk::KeyE => Pk::KeyE,
        Wk::KeyF => Pk::KeyF,
        Wk::KeyG => Pk::KeyG,
        Wk::KeyH => Pk::KeyH,
        Wk::KeyI => Pk::KeyI,
        Wk::KeyJ => Pk::KeyJ,
        Wk::KeyK => Pk::KeyK,
        Wk::KeyL => Pk::KeyL,
        Wk::KeyM => Pk::KeyM,
        Wk::KeyN => Pk::KeyN,
        Wk::KeyO => Pk::KeyO,
        Wk::KeyP => Pk::KeyP,
        Wk::KeyQ => Pk::KeyQ,
        Wk::KeyR => Pk::KeyR,
        Wk::KeyS => Pk::KeyS,
        Wk::KeyT => Pk::KeyT,
        Wk::KeyU => Pk::KeyU,
        Wk::KeyV => Pk::KeyV,
        Wk::KeyW => Pk::KeyW,
        Wk::KeyX => Pk::KeyX,
        Wk::KeyY => Pk::KeyY,
        Wk::KeyZ => Pk::KeyZ,
        Wk::F1 => Pk::F1,
        Wk::F2 => Pk::F2,
        Wk::F3 => Pk::F3,
        Wk::F4 => Pk::F4,
        Wk::F5 => Pk::F5,
        Wk::F6 => Pk::F6,
        Wk::F7 => Pk::F7,
        Wk::F8 => Pk::F8,
        Wk::F9 => Pk::F9,
        Wk::F10 => Pk::F10,
        Wk::F11 => Pk::F11,
        Wk::F12 => Pk::F12,
        Wk::Escape => Pk::Escape,
        Wk::Space => Pk::Space,
        Wk::Enter => Pk::Enter,
        Wk::Backspace => Pk::Backspace,
        Wk::Tab => Pk::Tab,
        Wk::Delete => Pk::Delete,
        Wk::ArrowUp => Pk::ArrowUp,
        Wk::ArrowDown => Pk::ArrowDown,
        Wk::ArrowLeft => Pk::ArrowLeft,
        Wk::ArrowRight => Pk::ArrowRight,
        Wk::Home => Pk::Home,
        Wk::End => Pk::End,
        Wk::PageUp => Pk::PageUp,
        Wk::PageDown => Pk::PageDown,
        Wk::Minus => Pk::Minus,
        Wk::Equal => Pk::Equal,
        Wk::BracketLeft => Pk::BracketLeft,
        Wk::BracketRight => Pk::BracketRight,
        Wk::Comma => Pk::Comma,
        Wk::Period => Pk::Period,
        Wk::Slash => Pk::Slash,
        Wk::Backslash => Pk::Backslash,
        Wk::Semicolon => Pk::Semicolon,
        Wk::Quote => Pk::Quote,
        Wk::Backquote => Pk::Backquote,
        _ => return None,
    })
}

/// Owns interaction state.
pub(super) struct Interaction {
    pub(super) input: InputBridge,
    pub(super) manipulation: ViewportManipulation,
    pub(super) last_hover_mode: Option<MouseSelectionMode>,
    pub(super) winit_modifiers: Cell<(bool, bool, bool, bool)>,
    pub(super) cursor_pos: Cell<(f32, f32)>,
    pub(super) cursor_known: Cell<bool>,
}
