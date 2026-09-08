use super::math::hash_view_proj;
use super::picking_budget::{plan_picking_budget, uses_lazy_manual_picking};
use super::state::PickingTextureSource;
use super::state::RepPickingState;
use super::RenderState;
use crate::byte_units::bytes_to_mib;
use crate::passes::lighting::identity_mat4;
use crate::picking::pass::PickingParams;
use crate::picking::readback::PickingReadback;
use crate::picking::readback::{PendingPick, PickReadbackTarget};
use crate::picking::reproject::ReprojectParams;
use crate::picking::{ObjectId, PickHit, PickingMode, RepKind};
use crate::render_input::{RenderInput, RenderObjectInput};
use crate::representations::catalog;

const REPROJECT_ANGULAR_THRESHOLD_RAD: f32 = 0.26;
const MAX_REPROJECT_FRAMES: u32 = 30;

pub(super) enum PickingStrategy {
    Skip,
    FullRecord,
    Reproject,
}

impl RenderState {
    pub(super) fn ensure_id_pass(&mut self) {
        if self.picking.id_pass.is_none() {
            self.picking.id_pass = Some(crate::picking::PickingPass::new(
                &self.ctx,
                &self.scene.scene_layout,
                &self.geometry.dot_params_layout,
            ));
        }

        let Some(id_pass) = self.picking.id_pass.as_ref() else {
            return;
        };
        for ((object_id, kind), entry) in self.scene.reps.iter_mut() {
            if entry.picking.is_some() {
                continue;
            }
            let params = PickingParams::new(*kind, ObjectId(*object_id));
            let (buffer, bind_group) = id_pass.make_params(&self.ctx.device, params);
            entry.picking = Some(RepPickingState {
                _buffer: buffer,
                bind_group,
            });
        }
    }

    pub(super) fn invalidate_picking_cache(&mut self) {
        self.picking.picking_pass_initialized = false;
        self.picking.picking_texture_source = None;
        self.picking.last_full_record_view_proj = None;
        self.picking.reproject_count = 0;
    }

    pub(super) fn invalidate_overlay_id_cache(&mut self) {
        self.picking.overlay_id_pass_initialized = false;
        self.picking.overlay_id_view_proj_hash = 0;
    }

    pub(super) fn sync_manual_picking(
        &mut self,
        input: &RenderInput<'_>,
        active_rep_count: usize,
    ) -> bool {
        if self.picking.picking_mode == PickingMode::Disabled
            || !uses_lazy_manual_picking(self.memory.policy)
        {
            return false;
        }

        let snapshot = self.memory_snapshot();
        let fixed_reserved_bytes = super::picking_budget::fixed_reserved_without_picking(&snapshot);
        let total_atoms = total_atom_count(input.objects);
        let decision = plan_picking_budget(
            self.memory.policy,
            fixed_reserved_bytes,
            (self.targets.width, self.targets.height),
            active_rep_count,
            total_atoms,
        );

        if !decision.allowed {
            if !self.memory.warned_picking_budget_denied {
                let headroom = decision
                    .budget_bytes
                    .unwrap_or(0)
                    .saturating_sub(decision.fixed_reserved_bytes);
                let message = format!(
                    "Hit-test picking disabled by render memory profile {}: estimated {:.2} MiB, headroom {:.2} MiB, budget {:.2} MiB, atoms {}, active reps {}.",
                    self.memory.policy.profile,
                    bytes_to_mib(decision.estimated_bytes),
                    bytes_to_mib(headroom),
                    bytes_to_mib(decision.budget_bytes.unwrap_or(0)),
                    decision.total_atoms,
                    decision.active_rep_count
                );
                log::warn!("patinae-render: {message}");
                self.memory.pending_warnings.push(message);
                self.memory.warned_picking_budget_denied = true;
            }
            return self.clear_hit_test_picking_resources();
        }

        self.memory.warned_picking_budget_denied = false;
        let was_allowed = self.picking.budget_allowed;
        let targets_allocated = self.targets.ensure_picking_targets(&self.ctx.device, false);
        let readback_allocated = ensure_picking_readbacks(&mut self.picking, &self.ctx.device);
        self.picking.budget_allowed = true;
        if !was_allowed || targets_allocated || readback_allocated {
            self.invalidate_picking_cache();
            return true;
        }
        false
    }

    pub(super) fn clear_hit_test_picking_resources(&mut self) -> bool {
        let had_resources = self.picking.budget_allowed
            || self.targets.picking_texture.is_some()
            || self.targets.picking_depth_texture.is_some()
            || self.targets.picking_prev_texture.is_some()
            || self.targets.picking_depth_prev_texture.is_some()
            || self.picking.readback.is_some()
            || self.picking.hover_readback.is_some()
            || self.picking.picking_reproject.is_some()
            || self.picking.id_pass.is_some()
            || self
                .scene
                .reps
                .values()
                .any(|entry| entry.picking.is_some());

        self.targets.clear_picking_targets();
        self.picking.readback = None;
        self.picking.hover_readback = None;
        self.picking.picking_reproject = None;
        self.picking.id_pass = None;
        self.picking.budget_allowed = false;
        for entry in self.scene.reps.values_mut() {
            entry.picking = None;
        }
        self.invalidate_picking_cache();
        had_resources
    }

    pub(super) fn pick_picking_strategy(
        &self,
        camera_changed: bool,
        has_marker: bool,
        allow_reproject: bool,
    ) -> PickingStrategy {
        if self.picking.picking_mode == PickingMode::Disabled {
            return PickingStrategy::Skip;
        }
        if !self.picking.budget_allowed {
            return PickingStrategy::Skip;
        }
        if !has_marker {
            return PickingStrategy::Skip;
        }
        if !self.picking.picking_pass_initialized {
            return PickingStrategy::FullRecord;
        }
        if !camera_changed {
            return PickingStrategy::Skip;
        }
        if !allow_reproject {
            return PickingStrategy::FullRecord;
        }

        match self.picking.picking_mode {
            PickingMode::FullRecord => PickingStrategy::FullRecord,
            PickingMode::Reprojected => {
                if self.picking.reproject_count >= MAX_REPROJECT_FRAMES {
                    return PickingStrategy::FullRecord;
                }
                if let Some(baseline) = self.picking.last_full_record_view_proj {
                    if angular_drift_exceeds(
                        &baseline,
                        &self.uniforms.view_proj,
                        REPROJECT_ANGULAR_THRESHOLD_RAD,
                    ) {
                        return PickingStrategy::FullRecord;
                    }
                }
                if self.picking.picking_reproject.is_some()
                    && self.targets.picking_prev.is_some()
                    && self.targets.picking_depth_prev.is_some()
                {
                    PickingStrategy::Reproject
                } else {
                    PickingStrategy::FullRecord
                }
            }
            PickingMode::Disabled => unreachable!("filtered earlier"),
        }
    }

    pub(super) fn refresh_overlay_id_texture(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view_proj_hash: u64,
    ) {
        if self.picking.overlay_id_pass_initialized
            && self.picking.overlay_id_view_proj_hash == view_proj_hash
        {
            return;
        }

        if self.picking.id_pass.is_none()
            || self.targets.overlay_id.is_none()
            || self.targets.overlay_id_depth.is_none()
        {
            self.invalidate_overlay_id_cache();
            return;
        }

        self.record_overlay_id_pass(encoder);
        self.picking.overlay_id_pass_initialized = true;
        self.picking.overlay_id_view_proj_hash = view_proj_hash;
    }

    pub(super) fn snapshot_picking_to_prev(&self, encoder: &mut wgpu::CommandEncoder) {
        let (Some(picking), Some(prev), Some(depth), Some(depth_prev)) = (
            self.targets.picking_texture.as_ref(),
            self.targets.picking_prev_texture.as_ref(),
            self.targets.picking_depth_texture.as_ref(),
            self.targets.picking_depth_prev_texture.as_ref(),
        ) else {
            return;
        };
        let (pw, ph) = self.targets.picking_dims();
        let extent = wgpu::Extent3d {
            width: pw,
            height: ph,
            depth_or_array_layers: 1,
        };
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: picking,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: prev,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            extent,
        );
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: depth,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            wgpu::TexelCopyTextureInfo {
                texture: depth_prev,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            extent,
        );
    }

    pub(super) fn dispatch_reproject(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(reproject) = self.picking.picking_reproject.as_mut() else {
            return;
        };
        let (Some(picking_prev), Some(depth_prev), Some(picking_cur)) = (
            self.targets.picking_prev.as_ref(),
            self.targets.picking_depth_prev.as_ref(),
            self.targets.picking.as_ref(),
        ) else {
            return;
        };
        let Some(baseline) = self.picking.last_full_record_view_proj else {
            return;
        };

        let (pw, ph) = self.targets.picking_dims();
        reproject.ensure_best_depth_capacity(&self.ctx.device, pw, ph);

        let params = ReprojectParams {
            view_proj_prev_inv: invert_mat4(&baseline).unwrap_or_else(identity_mat4),
            view_proj_cur: self.uniforms.view_proj,
        };
        reproject.upload_params(&self.ctx.queue, params);
        reproject.clear_best_depth(&self.ctx.queue, pw, ph);

        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("patinae.picking.reproject.clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: picking_cur,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        let bind_group =
            reproject.make_bind_group(&self.ctx.device, picking_prev, depth_prev, picking_cur);
        reproject.record(encoder, &bind_group, pw, ph);
    }

    pub(super) fn record_picking_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let (Some(id_pass), Some(picking_view), Some(picking_depth_view)) = (
            self.picking.id_pass.as_ref(),
            self.targets.picking.as_ref(),
            self.targets.picking_depth.as_ref(),
        ) else {
            return;
        };
        self.record_id_pass_to(
            encoder,
            id_pass,
            picking_view,
            picking_depth_view,
            "patinae.picking_pass",
        );
    }

    pub(super) fn record_overlay_id_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let (Some(id_pass), Some(overlay_view), Some(overlay_depth_view)) = (
            self.picking.id_pass.as_ref(),
            self.targets.overlay_id.as_ref(),
            self.targets.overlay_id_depth.as_ref(),
        ) else {
            return;
        };
        self.record_id_pass_to(
            encoder,
            id_pass,
            overlay_view,
            overlay_depth_view,
            "patinae.overlay_id_pass",
        );
    }

    fn record_id_pass_to(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        id_pass: &crate::picking::PickingPass,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        label: &'static str,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        pass.set_bind_group(0, &self.ctx.frame.bind_group, &[]);
        pass.set_bind_group(1, &self.ctx.lighting.bind_group, &[]);
        let mut current: Option<RepKind> = None;
        for key in &self.scene.draw_order {
            let Some(entry) = self.scene.reps.get(key) else {
                continue;
            };
            let kind = entry.rep.kind();
            if current != Some(kind) {
                let Some(pipeline) =
                    catalog::entry(kind).and_then(|rep| rep.picking_pipeline(id_pass))
                else {
                    continue;
                };
                pass.set_pipeline(pipeline);
                current = Some(kind);
            }
            let Some(rep_picking) = entry.picking.as_ref() else {
                continue;
            };
            pass.set_bind_group(2, &rep_picking.bind_group, &[]);
            let store = &self.scene.scene_store;
            let Some(bg) = store.bind_group() else {
                continue;
            };
            let group = if kind == RepKind::Dot { 1 } else { 3 };
            // Dot parameters occupy group 3; its scene data uses group 1.
            if kind != RepKind::Dot {
                pass.set_bind_group(1, &self.ctx.lighting.bind_group, &[]);
            }
            for &offset in store.draw_offsets(ObjectId(key.0)) {
                pass.set_bind_group(group, bg, &[offset]);
                if store.needs_raw_draw(ObjectId(key.0)) {
                    entry.rep.record_raw_picking(&mut pass);
                } else {
                    entry.rep.record_picking(&mut pass);
                }
            }
        }
    }

    pub fn submit_pick(&mut self, x: u32, y: u32) -> Option<PendingPick> {
        self.submit_pick_with_target(x, y, PickReadbackTarget::Hover)
    }

    pub fn submit_pick_with_target(
        &mut self,
        x: u32,
        y: u32,
        target: PickReadbackTarget,
    ) -> Option<PendingPick> {
        if !self.hit_test_picking_available() {
            return None;
        }
        self.ensure_id_pass();
        let pick_x = ((x as f32) * self.targets.picking_scale) as u32;
        let pick_y = ((y as f32) * self.targets.picking_scale) as u32;
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("patinae.pick.submit"),
            });
        self.refresh_picking_for_pick(&mut encoder);

        let readback = match target {
            PickReadbackTarget::Hover => self.picking.hover_readback.as_ref()?,
            PickReadbackTarget::Click => self.picking.readback.as_ref()?,
        };
        let picking_texture = self.targets.picking_texture.as_ref()?;
        let pending = readback.issue_copy(&mut encoder, picking_texture, pick_x, pick_y, target)?;
        self.ctx.queue.submit(std::iter::once(encoder.finish()));
        readback.map_async(&pending);
        Some(pending)
    }

    fn refresh_picking_for_pick(&mut self, encoder: &mut wgpu::CommandEncoder) {
        self.prepare_instance_draws(encoder);
        let view_proj_hash = hash_view_proj(&self.uniforms.view_proj);
        let camera_changed = view_proj_hash != self.picking.last_view_proj_hash;
        if self.picking.picking_pass_initialized && !camera_changed {
            return;
        }

        let prev_marker = self.scene.has_any_marker;
        self.scene.has_any_marker = true;
        let strategy = self.pick_picking_strategy(camera_changed, true, true);
        self.scene.has_any_marker = prev_marker;

        match strategy {
            PickingStrategy::Skip => {}
            PickingStrategy::FullRecord => {
                self.record_picking_pass(encoder);
                self.snapshot_picking_to_prev(encoder);
                self.picking.last_full_record_view_proj = Some(self.uniforms.view_proj);
                self.picking.reproject_count = 0;
                self.picking.picking_texture_source = Some(PickingTextureSource::FullRecord);
            }
            PickingStrategy::Reproject => {
                self.dispatch_reproject(encoder);
                self.picking.reproject_count = self.picking.reproject_count.saturating_add(1);
                self.picking.picking_texture_source = Some(PickingTextureSource::Reprojected);
            }
        }
        self.picking.picking_pass_initialized = true;
        self.picking.last_view_proj_hash = view_proj_hash;
    }

    pub fn try_collect_pick(&self, pending: &PendingPick) -> Option<Option<PickHit>> {
        if !self.hit_test_picking_available() {
            return Some(None);
        }
        if !pending.is_ready() {
            return None;
        }
        let readback = match pending.target() {
            PickReadbackTarget::Hover => self.picking.hover_readback.as_ref(),
            PickReadbackTarget::Click => self.picking.readback.as_ref(),
        };
        let Some(readback) = readback else {
            return Some(None);
        };
        readback.try_collect(pending)
    }

    pub fn pick(&mut self, x: u32, y: u32) -> Option<PickHit> {
        if !self.hit_test_picking_available() {
            return None;
        }
        self.ensure_id_pass();
        let pick_x = ((x as f32) * self.targets.picking_scale) as u32;
        let pick_y = ((y as f32) * self.targets.picking_scale) as u32;

        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("patinae.pick.copy"),
            });
        self.refresh_picking_for_pick(&mut encoder);

        let readback = self.picking.readback.as_ref()?;
        let picking_texture = self.targets.picking_texture.as_ref()?;
        let pending = readback.issue_copy(
            &mut encoder,
            picking_texture,
            pick_x,
            pick_y,
            PickReadbackTarget::Click,
        )?;
        self.ctx.queue.submit(std::iter::once(encoder.finish()));

        readback.map_async(&pending);
        loop {
            self.ctx
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .expect("device poll");
            if pending.is_ready() {
                break;
            }
        }
        readback.try_collect(&pending).flatten()
    }

    fn hit_test_picking_available(&self) -> bool {
        self.picking.picking_mode != PickingMode::Disabled
            && self.picking.budget_allowed
            && self.targets.picking_texture.is_some()
            && self.targets.picking_depth_texture.is_some()
            && self.picking.readback.is_some()
            && self.picking.hover_readback.is_some()
    }
}

fn ensure_picking_readbacks(
    picking: &mut super::state::PickingRuntime,
    device: &wgpu::Device,
) -> bool {
    let mut allocated = false;
    if picking.readback.is_none() {
        picking.readback = Some(PickingReadback::new(device));
        allocated = true;
    }
    if picking.hover_readback.is_none() {
        picking.hover_readback = Some(PickingReadback::new(device));
        allocated = true;
    }
    allocated
}

fn total_atom_count(objects: &[RenderObjectInput<'_>]) -> u64 {
    objects
        .iter()
        .map(|object| object.molecule.atom_count() as u64)
        .sum()
}

fn invert_mat4(m: &[[f32; 4]; 4]) -> Option<[[f32; 4]; 4]> {
    let a = m;
    let b00 = a[0][0] * a[1][1] - a[1][0] * a[0][1];
    let b01 = a[0][0] * a[1][2] - a[1][0] * a[0][2];
    let b02 = a[0][0] * a[1][3] - a[1][0] * a[0][3];
    let b03 = a[0][1] * a[1][2] - a[1][1] * a[0][2];
    let b04 = a[0][1] * a[1][3] - a[1][1] * a[0][3];
    let b05 = a[0][2] * a[1][3] - a[1][2] * a[0][3];
    let b06 = a[2][0] * a[3][1] - a[3][0] * a[2][1];
    let b07 = a[2][0] * a[3][2] - a[3][0] * a[2][2];
    let b08 = a[2][0] * a[3][3] - a[3][0] * a[2][3];
    let b09 = a[2][1] * a[3][2] - a[3][1] * a[2][2];
    let b10 = a[2][1] * a[3][3] - a[3][1] * a[2][3];
    let b11 = a[2][2] * a[3][3] - a[3][2] * a[2][3];
    let det = b00 * b11 - b01 * b10 + b02 * b09 + b03 * b08 - b04 * b07 + b05 * b06;
    if det.abs() < 1e-12 {
        return None;
    }
    let inv_det = 1.0 / det;
    Some([
        [
            (a[1][1] * b11 - a[1][2] * b10 + a[1][3] * b09) * inv_det,
            (a[0][2] * b10 - a[0][1] * b11 - a[0][3] * b09) * inv_det,
            (a[3][1] * b05 - a[3][2] * b04 + a[3][3] * b03) * inv_det,
            (a[2][2] * b04 - a[2][1] * b05 - a[2][3] * b03) * inv_det,
        ],
        [
            (a[1][2] * b08 - a[1][0] * b11 - a[1][3] * b07) * inv_det,
            (a[0][0] * b11 - a[0][2] * b08 + a[0][3] * b07) * inv_det,
            (a[3][2] * b02 - a[3][0] * b05 - a[3][3] * b01) * inv_det,
            (a[2][0] * b05 - a[2][2] * b02 + a[2][3] * b01) * inv_det,
        ],
        [
            (a[1][0] * b10 - a[1][1] * b08 + a[1][3] * b06) * inv_det,
            (a[0][1] * b08 - a[0][0] * b10 - a[0][3] * b06) * inv_det,
            (a[3][0] * b04 - a[3][1] * b02 + a[3][3] * b00) * inv_det,
            (a[2][1] * b02 - a[2][0] * b04 - a[2][3] * b00) * inv_det,
        ],
        [
            (a[1][1] * b07 - a[1][0] * b09 - a[1][2] * b06) * inv_det,
            (a[0][0] * b09 - a[0][1] * b07 + a[0][2] * b06) * inv_det,
            (a[3][1] * b01 - a[3][0] * b03 - a[3][2] * b00) * inv_det,
            (a[2][0] * b03 - a[2][1] * b01 + a[2][2] * b00) * inv_det,
        ],
    ])
}

fn angular_drift_exceeds(
    baseline: &[[f32; 4]; 4],
    current: &[[f32; 4]; 4],
    threshold_rad: f32,
) -> bool {
    let f0 = [baseline[0][2], baseline[1][2], baseline[2][2]];
    let f1 = [current[0][2], current[1][2], current[2][2]];
    let n0 = (f0[0] * f0[0] + f0[1] * f0[1] + f0[2] * f0[2])
        .sqrt()
        .max(1e-6);
    let n1 = (f1[0] * f1[0] + f1[1] * f1[1] + f1[2] * f1[2])
        .sqrt()
        .max(1e-6);
    let cos_theta = (f0[0] * f1[0] + f0[1] * f1[1] + f0[2] * f1[2]) / (n0 * n1);
    let cos_theta = cos_theta.clamp(-1.0, 1.0);
    cos_theta.acos() > threshold_rad
}
