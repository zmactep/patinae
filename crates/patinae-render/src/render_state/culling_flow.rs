use crate::compute::cull::frustum_planes_from_view_proj;
use crate::picking::ObjectId;
use crate::representations::catalog;
use crate::representations::{CullPlanCtx, ViewportLodCtx};
#[cfg(feature = "stats")]
use crate::stats::Pass as StatsPass;

use super::RenderState;

impl RenderState {
    pub(super) fn invalidate_cull_cache(&mut self) {
        self.scene.cull_pass_initialized = false;
        self.scene.last_cull_view_proj_hash = 0;
    }

    pub(super) fn poll_viewport_lod(&mut self) {
        let queue = self.ctx.queue.clone();
        let mut changed = false;
        for entry in self.scene.reps.values_mut() {
            if entry.rep.poll_viewport_lod(&queue) {
                entry.draw_phase = entry.rep.draw_phase();
                changed = true;
            }
        }

        let _ = self.ctx.device.poll(wgpu::PollType::Poll);

        for entry in self.scene.reps.values_mut() {
            if entry.rep.poll_viewport_lod(&queue) {
                entry.draw_phase = entry.rep.draw_phase();
                changed = true;
            }
        }

        if changed {
            self.scene.scene_dirty = true;
            self.invalidate_cull_cache();
            self.invalidate_picking_cache();
            self.invalidate_overlay_id_cache();
        }
    }

    /// Dispatch the per-rep cull kernel when the cached compacted buffers
    /// no longer match the scene or camera.
    pub(super) fn dispatch_cull(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view_proj_hash: u64,
        compute_rebuilt: bool,
    ) {
        if !should_dispatch_cull(
            self.scene.cull_pass_initialized,
            self.scene.scene_dirty,
            compute_rebuilt,
            self.scene.last_cull_view_proj_hash,
            view_proj_hash,
        ) {
            return;
        }

        let view_proj = self.uniforms.view_proj;
        let frustum_planes = frustum_planes_from_view_proj(&view_proj);
        let queue = self.ctx.queue.clone();
        let plan_ctx = CullPlanCtx {
            queue: &queue,
            view_proj,
            frustum_planes,
        };
        let mut dispatched: u32 = 0;

        for entry in self.scene.reps.values_mut() {
            let kind = entry.rep.kind();
            let Some(plan) = entry.rep.plan_cull(&plan_ctx) else {
                continue;
            };
            let Some(rep_catalog) = catalog::entry(kind).filter(|rep| rep.cullable) else {
                continue;
            };
            let Some(pipeline) = rep_catalog.cull_pipeline(&self.geometry.cull_pipeline) else {
                continue;
            };
            let label = rep_catalog.cull_label();
            let ts: Option<wgpu::ComputePassTimestampWrites<'_>> = {
                #[cfg(feature = "stats")]
                {
                    if dispatched == 0 {
                        self.stats.compute_pass_timestamp_writes(StatsPass::Cull)
                    } else {
                        None
                    }
                }
                #[cfg(not(feature = "stats"))]
                {
                    let _ = dispatched;
                    None
                }
            };
            self.geometry.cull_pipeline.dispatch_kind(
                encoder,
                pipeline,
                plan.bind_group,
                plan.upper,
                label,
                ts,
            );
            dispatched += 1;
        }

        self.scene.cull_pass_initialized = true;
        self.scene.last_cull_view_proj_hash = view_proj_hash;
    }

    pub(super) fn record_viewport_lod_readbacks(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let Some(scene_bg) = self.scene.scene_store.bind_group().cloned() else {
            return;
        };
        for (key, entry) in self.scene.reps.iter_mut() {
            let Some(slot) = self.scene.scene_store.slot(ObjectId(key.0)) else {
                continue;
            };
            let mut ctx = ViewportLodCtx {
                encoder,
                scene_bg: &scene_bg,
                obj_dynamic_offset: slot.dynamic_offset(),
                pipelines: &self.geometry,
            };
            entry.rep.record_viewport_lod_readback(&mut ctx);
        }
    }
}

fn should_dispatch_cull(
    cull_pass_initialized: bool,
    scene_dirty: bool,
    compute_rebuilt: bool,
    last_view_proj_hash: u64,
    view_proj_hash: u64,
) -> bool {
    !cull_pass_initialized
        || scene_dirty
        || compute_rebuilt
        || last_view_proj_hash != view_proj_hash
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a real GPU; exercises marker-only and coordinate changes through sync"]
    fn gpu_sync_markers_reuse_culling_and_coordinates_invalidate_it() {
        use crate::{
            RenderAtomColors, RenderInput, RenderObjectInput, SceneLod, IDENTITY_TRANSFORM,
        };
        use lin_alg::f32::Vec3;
        use patinae_mol::{Atom, AtomIndex, DirtyFlags, Element, MoleculeBuilder, RepMask};
        use patinae_settings::{ResolvedSettings, Settings};
        use std::sync::Arc;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("GPU adapter required; do not silently skip");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: crate::required_limits_for_memory_policy(
                &adapter.limits(),
                crate::RenderMemoryPolicy::performance(),
            ),
            ..Default::default()
        }))
        .unwrap();
        let mut renderer = RenderState::with_config(
            Arc::new(device),
            Arc::new(queue),
            wgpu::TextureFormat::Rgba8Unorm,
            (32, 32),
            Default::default(),
        );
        let mut molecule = MoleculeBuilder::new("cull-markers")
            .add_atom(Atom::new("CA", Element::Carbon), Vec3::new(0.0, 0.0, 0.0))
            .build();
        let settings = ResolvedSettings::resolve(&Settings::default(), None);
        let colors = [[1.0; 4]];
        for (dirty, marker, expect_dirty) in [
            (DirtyFlags::ALL, 0, true),
            (
                DirtyFlags::SELECTION,
                crate::scene_store::marker::MARKER_SELECTED,
                false,
            ),
            (
                DirtyFlags::HOVER,
                crate::scene_store::marker::MARKER_HOVER,
                false,
            ),
            (DirtyFlags::COORDS, 0, true),
        ] {
            if dirty == DirtyFlags::COORDS {
                assert!(molecule.set_coord(AtomIndex(0), 0, Vec3::new(2.0, 0.0, 0.0)));
            }
            let markers = [marker];
            let object = RenderObjectInput {
                object_id: ObjectId(1),
                instances: None,
                molecule: &molecule,
                coord_set: molecule.get_coord_set(0).unwrap(),
                transform: IDENTITY_TRANSFORM,
                visible_reps: RepMask::SPHERES,
                draw_reps: RepMask::SPHERES,
                object_settings: None,
                colors: RenderAtomColors::Separate {
                    base: &colors,
                    reps: &[],
                },
                atom_markers: &markers,
                recent_atom_markers: None,
                marker_updates: &[],
                has_markers: marker != 0,
                lod: SceneLod::Auto,
                dirty,
            };
            renderer.sync(&RenderInput {
                objects: &[object],
                maps: &[],
                strokes: &[],
                settings: &settings,
                lod: SceneLod::Auto,
            });
            assert_eq!(renderer.scene.scene_dirty, expect_dirty, "{dirty:?}");
            assert_eq!(
                should_dispatch_cull(
                    renderer.scene.cull_pass_initialized,
                    renderer.scene.scene_dirty,
                    false,
                    renderer.scene.last_cull_view_proj_hash,
                    7
                ),
                expect_dirty,
                "{dirty:?}"
            );
            let mut encoder = renderer
                .ctx
                .device
                .create_command_encoder(&Default::default());
            renderer.dispatch_cull(&mut encoder, 7, false);
            renderer.ctx.queue.submit([encoder.finish()]);
            assert!(renderer.scene.cull_pass_initialized);
            assert_eq!(renderer.scene.last_cull_view_proj_hash, 7);
            // A completed frame consumes the scene dirty bit.
            renderer.scene.scene_dirty = false;
        }
    }

    #[test]
    fn frustum_plane_extraction_returns_normalized_planes() {
        let planes = frustum_planes_from_view_proj(&[
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ]);
        for plane in planes {
            let len = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5);
            assert!(plane.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn cull_cache_dispatches_first_frame() {
        assert!(should_dispatch_cull(false, false, false, 7, 7));
    }

    #[test]
    fn cull_cache_reuses_static_frame() {
        assert!(!should_dispatch_cull(true, false, false, 7, 7));
    }

    #[test]
    fn cull_cache_dispatches_when_camera_changes() {
        assert!(should_dispatch_cull(true, false, false, 7, 8));
    }

    #[test]
    fn cull_cache_dispatches_when_scene_dirty() {
        assert!(should_dispatch_cull(true, true, false, 7, 7));
    }

    #[test]
    fn cull_cache_dispatches_when_compute_rebuilt() {
        assert!(should_dispatch_cull(true, false, true, 7, 7));
    }
}
