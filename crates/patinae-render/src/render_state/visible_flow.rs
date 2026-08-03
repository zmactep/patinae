use super::state::*;
use crate::picking::{ObjectId, RepKind};
use crate::render_input::RenderMapMode;
use crate::representations::catalog;
use crate::representations::DrawPhase;
use crate::scene_store::SceneStore;
#[cfg(feature = "stats")]
use crate::stats::Pass as StatsPass;

impl RenderState {
    pub(super) fn count_wboit_reps(&self) -> u32 {
        self.scene
            .draw_order
            .iter()
            .filter_map(|k| self.scene.reps.get(k))
            .filter(|entry| entry.draw_phase == DrawPhase::Wboit)
            .count() as u32
            + self
                .scene
                .map_draw_order
                .iter()
                .filter_map(|id| self.scene.maps.get(id))
                .filter(|entry| !entry.is_opaque)
                .count() as u32
    }

    pub(super) fn count_fast_overlay_reps(&self) -> u32 {
        self.scene
            .draw_order
            .iter()
            .filter_map(|k| self.scene.reps.get(k))
            .filter(|entry| entry.draw_phase == DrawPhase::FastOverlay)
            .count() as u32
    }

    pub(super) fn record_opaque_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        #[cfg(feature = "stats")]
        let opaque_ts = self.stats.render_pass_timestamp_writes(StatsPass::Opaque);
        #[cfg(not(feature = "stats"))]
        let opaque_ts = None;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("patinae.opaque_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(self.screen.clear_color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: opaque_ts,
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
            if entry.draw_phase != DrawPhase::Opaque {
                continue;
            }
            let kind = entry.rep.kind();
            if current != Some(kind) {
                let Some(pipeline) = catalog::entry(kind)
                    .and_then(|rep| rep.color_pipeline(&self.geometry, DrawPhase::Opaque))
                else {
                    continue;
                };
                pass.set_pipeline(pipeline);
                current = Some(kind);
            }
            if !bind_group_2(&mut pass, kind, key.0, entry, &self.scene.scene_store) {
                continue;
            }
            entry.rep.record_translucent(&mut pass);
        }
        self.record_maps_opaque(&mut pass);
    }

    pub(super) fn record_translucent_pass(&self, encoder: &mut wgpu::CommandEncoder, n_wboit: u32) {
        if n_wboit == 0 {
            return;
        }
        #[cfg(feature = "stats")]
        let translucent_ts = self
            .stats
            .render_pass_timestamp_writes(StatsPass::Translucent);
        #[cfg(not(feature = "stats"))]
        let translucent_ts = None;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("patinae.translucent_pass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.accum,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.reveal,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: translucent_ts,
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
            if entry.draw_phase != DrawPhase::Wboit {
                continue;
            }
            let kind = entry.rep.kind();
            if current != Some(kind) {
                let Some(pipeline) = catalog::entry(kind)
                    .and_then(|rep| rep.color_pipeline(&self.geometry, DrawPhase::Wboit))
                else {
                    continue;
                };
                pass.set_pipeline(pipeline);
                current = Some(kind);
            }
            if !bind_group_2(&mut pass, kind, key.0, entry, &self.scene.scene_store) {
                continue;
            }
            entry.rep.record_translucent(&mut pass);
        }
        self.record_maps_translucent(&mut pass);
    }

    pub(super) fn record_fast_overlay_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        n_fast_overlay: u32,
    ) {
        if n_fast_overlay == 0 {
            return;
        }
        #[cfg(feature = "stats")]
        let fast_overlay_ts = self
            .stats
            .render_pass_timestamp_writes(StatsPass::FastOverlay);
        #[cfg(not(feature = "stats"))]
        let fast_overlay_ts = None;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("patinae.fast_overlay_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: fast_overlay_ts,
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
            if entry.draw_phase != DrawPhase::FastOverlay {
                continue;
            }
            let kind = entry.rep.kind();
            if current != Some(kind) {
                let Some(pipeline) = catalog::entry(kind)
                    .and_then(|rep| rep.color_pipeline(&self.geometry, DrawPhase::FastOverlay))
                else {
                    continue;
                };
                pass.set_pipeline(pipeline);
                current = Some(kind);
            }
            if !bind_group_2(&mut pass, kind, key.0, entry, &self.scene.scene_store) {
                continue;
            }
            entry.rep.record_translucent(&mut pass);
        }
    }

    pub(super) fn record_wboit_composite(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        n_wboit: u32,
    ) {
        if n_wboit == 0 {
            return;
        }
        #[cfg(feature = "stats")]
        let composite_ts = self
            .stats
            .render_pass_timestamp_writes(StatsPass::Composite);
        #[cfg(not(feature = "stats"))]
        let composite_ts = None;

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("patinae.wboit_composite_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: composite_ts,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        pass.set_pipeline(&self.screen.composite.pipeline);
        pass.set_bind_group(0, &self.screen.composite_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub(super) fn record_stroke_overlay(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
    ) {
        if self.scene.stroke_draw_order.is_empty() {
            return;
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("patinae.stroke_overlay_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.targets.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.geometry.stroke_pipeline.pipeline);
        pass.set_bind_group(0, &self.ctx.frame.bind_group, &[]);
        for object_id in &self.scene.stroke_draw_order {
            if let Some(entry) = self.scene.strokes.get(object_id) {
                entry.record(&mut pass);
            }
        }
    }
}

impl RenderState {
    fn record_maps_opaque<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let mut current: Option<RenderMapMode> = None;
        for object_id in &self.scene.map_draw_order {
            let Some(entry) = self.scene.maps.get(object_id) else {
                continue;
            };
            if !entry.is_opaque {
                continue;
            }
            if current != Some(entry.mode) {
                let pipeline = match entry.mode {
                    RenderMapMode::Isomesh => &self.geometry.map_pipeline.line_opaque,
                    RenderMapMode::Isosurface => &self.geometry.map_pipeline.triangles_opaque,
                };
                pass.set_pipeline(pipeline);
                current = Some(entry.mode);
            }
            entry.record(pass);
        }
    }

    fn record_maps_translucent<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        let mut current: Option<RenderMapMode> = None;
        for object_id in &self.scene.map_draw_order {
            let Some(entry) = self.scene.maps.get(object_id) else {
                continue;
            };
            if entry.is_opaque {
                continue;
            }
            if current != Some(entry.mode) {
                let pipeline = match entry.mode {
                    RenderMapMode::Isomesh => &self.geometry.map_pipeline.line,
                    RenderMapMode::Isosurface => &self.geometry.map_pipeline.triangles,
                };
                pass.set_pipeline(pipeline);
                current = Some(entry.mode);
            }
            entry.record(pass);
        }
    }
}

/// Bind group 2 for the rep being drawn. All reps live on `SceneStore`
/// (scene-wide bind group + per-object dynamic offset). Returns `false`
/// when no valid binding is available — the caller skips the draw.
pub(super) fn bind_group_2<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    _kind: RepKind,
    object_id: u32,
    _entry: &'pass RepEntry,
    scene_store: &'pass SceneStore,
) -> bool {
    let Some(slot) = scene_store.slot(ObjectId(object_id)) else {
        return false;
    };
    let Some(bg) = scene_store.bind_group() else {
        return false;
    };
    pass.set_bind_group(2, bg, &[slot.dynamic_offset()]);
    true
}
