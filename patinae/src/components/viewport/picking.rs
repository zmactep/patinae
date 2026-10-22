//! Picking for ViewportRenderer.

use super::*;

impl ViewportRenderer {
    /// Async hover pick: skip while a previous hover readback is in flight.
    /// - `Some(Some(hit))` — fresh hit landed this frame.
    /// - `Some(None)` — fresh miss landed this frame.
    /// - `None` — no fresh result (one is in flight or being submitted).
    pub(crate) fn poll_hover_pick(
        &mut self,
        session: &Session,
        x: u32,
        y: u32,
    ) -> Option<Option<PickHit>> {
        let (width, height) = self.last_viewport_size?;
        if x >= width || y >= height {
            return None;
        }
        self.refresh_pick_frame_uniforms(session, (width, height));

        if let Some(pending) = self.pending_hover.as_ref() {
            // Wait for the GPU to finish; never submit a second pick.
            let result = self.state.try_collect_pick(pending)?;
            self.pending_hover = None;
            return Some(
                result.and_then(|hit| resolve_pick(hit, self.render_scene.object_names(), session)),
            );
        }

        // No pick in flight — submit a fresh one. `submit_pick` returns
        // `None` if the renderer was built with `PickingMode::Disabled`;
        // in that case there's nothing to wait on.
        if self.large_lod_hover_pick_waiting() {
            return None;
        }
        self.pending_hover = self.state.submit_pick(x, y);
        if self.pending_hover.is_some() {
            self.last_hover_submit = Some(Instant::now());
        }
        None
    }

    pub(super) fn large_lod_hover_pick_waiting(&self) -> bool {
        (self.state.sphere_lod_diagnostics().active || self.state.stick_lod_diagnostics().active)
            && self
                .last_hover_submit
                .is_some_and(|last| last.elapsed() < LARGE_LOD_HOVER_PICK_INTERVAL)
    }

    /// Synchronous click pick. Uses a separate renderer readback from hover,
    /// so it never waits for a pending hover query before resolving a click.
    pub(crate) fn pick_at_click(&mut self, session: &Session, x: u32, y: u32) -> Option<PickHit> {
        let (width, height) = self.last_viewport_size?;
        if x >= width || y >= height {
            return None;
        }
        self.refresh_pick_frame_uniforms(session, (width, height));

        let hit = self.state.pick(x, y)?;
        resolve_pick(hit, self.render_scene.object_names(), session)
    }

    pub(super) fn refresh_pick_frame_uniforms(&mut self, session: &Session, viewport: (u32, u32)) {
        let frame = frame_uniforms_from_session(session, viewport, session.clear_color);
        self.state.uniforms = frame;
        self.state.ctx.upload_frame(&self.state.uniforms);
    }
}
