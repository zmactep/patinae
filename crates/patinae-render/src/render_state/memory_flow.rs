use super::state::RenderState;
use crate::memory::{buffer_usage, GpuMemoryCategory, GpuMemoryLedger, GpuMemorySnapshot};
use crate::{RenderMemoryPolicy, RepBudgetDiagnostic, SceneStoreCompactionStats};

impl RenderState {
    /// Returns the active renderer memory policy.
    pub fn memory_policy(&self) -> RenderMemoryPolicy {
        self.memory.policy
    }

    /// Drains pending user-visible renderer memory warnings.
    pub fn take_memory_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.memory.pending_warnings)
    }

    /// Force SceneStore compaction before an OOM retry.
    pub fn force_scene_store_compaction(&mut self) -> SceneStoreCompactionStats {
        let compaction = self.scene.scene_store.compact();
        if compaction.ran {
            self.scene.scene_dirty = true;
            self.screen.marking_offsets_dirty = true;
            self.screen.marking_bindings_dirty = true;
            self.memory.rep_budget_request_cache.clear();
            self.invalidate_cull_cache();
            self.invalidate_picking_cache();
            self.invalidate_overlay_id_cache();
        }
        compaction
    }

    /// Drop optional render targets that are not required by active settings.
    pub fn drop_unused_optional_targets(&mut self) {
        if !self.screen.ssao_enabled {
            self.targets.clear_ssao_targets();
            self.screen.ssao_bind_group = None;
            self.screen.ssao_blur_h_bind_group = None;
            self.screen.ssao_blur_v_bind_group = None;
            self.screen.ssao_compose_bind_group = None;
        }

        if !self.screen.fxaa_enabled {
            self.targets.clear_color_scratch();
            self.screen.fxaa_bind_group = None;
            self.screen.fxaa_overlay_bind_group = None;
        }

        if !self.screen.selection_overlay_enabled {
            self.screen.marking_bind_groups = None;
            self.targets.clear_marking_targets();
        }

        if self.screen.silhouette_params.is_none() && !self.screen.selection_overlay_enabled {
            self.screen.silhouette_bind_group = None;
            self.targets.clear_overlay_targets();
            self.invalidate_overlay_id_cache();
        }
    }

    /// Returns diagnostics from the most recent representation budget planning pass.
    pub fn last_rep_budget_diagnostics(&self) -> &[RepBudgetDiagnostic] {
        &self.memory.rep_budget_diagnostics
    }

    /// Returns a deterministic estimated GPU-memory snapshot.
    pub fn memory_snapshot(&self) -> GpuMemorySnapshot {
        let mut ledger = GpuMemoryLedger::new();

        self.targets.record_memory(&mut ledger);
        ledger.add_usage(
            GpuMemoryCategory::FrameTargets,
            buffer_usage(&self.ctx.frame.buffer),
        );
        ledger.add_usage(
            GpuMemoryCategory::SceneStore,
            self.scene.scene_store.memory_usage(),
        );

        for entry in self.scene.reps.values() {
            ledger.add_usage(GpuMemoryCategory::Representation, entry.rep.memory_usage());
            if let Some(picking) = entry.picking.as_ref() {
                ledger.add_usage(GpuMemoryCategory::Picking, buffer_usage(&picking._buffer));
            }
        }
        for map in self.scene.maps.values() {
            ledger.add_usage(GpuMemoryCategory::Representation, map.memory_usage());
        }
        for stroke in self.scene.strokes.values() {
            ledger.add_usage(GpuMemoryCategory::Overlay, stroke.memory_usage());
        }

        ledger.add_usage(
            GpuMemoryCategory::RepresentationScratch,
            self.geometry.dot_params_layout.memory_usage(),
        );
        ledger.add_usage(
            GpuMemoryCategory::SurfaceScratch,
            buffer_usage(&self.geometry.surface_mc_compute.tables_buf),
        );

        if let Some(readback) = self.picking.readback.as_ref() {
            ledger.add_usage(GpuMemoryCategory::Readback, readback.memory_usage());
        }
        if let Some(readback) = self.picking.hover_readback.as_ref() {
            ledger.add_usage(GpuMemoryCategory::Readback, readback.memory_usage());
        }
        if let Some(reproject) = self.picking.picking_reproject.as_ref() {
            ledger.add_usage(GpuMemoryCategory::Picking, reproject.memory_usage());
        }

        if let Some(marking) = self.screen.marking.as_ref() {
            ledger.add_usage(
                GpuMemoryCategory::Overlay,
                buffer_usage(&marking.params_buffer),
            );
            ledger.add_usage(
                GpuMemoryCategory::Overlay,
                buffer_usage(&marking.object_offsets_buffer),
            );
        }
        if let Some(silhouette) = self.screen.silhouette.as_ref() {
            ledger.add_usage(
                GpuMemoryCategory::Overlay,
                buffer_usage(&silhouette.uniform_buffer),
            );
        }
        ledger.add_usage(
            GpuMemoryCategory::Overlay,
            self.screen.atom_markers.memory_usage(),
        );

        ledger.add_usage(
            GpuMemoryCategory::Postprocess,
            buffer_usage(&self.screen.ssao_resources.ssao_params_buffer),
        );
        ledger.add_usage(
            GpuMemoryCategory::Postprocess,
            buffer_usage(&self.screen.ssao_resources.blur_h_params_buffer),
        );
        ledger.add_usage(
            GpuMemoryCategory::Postprocess,
            buffer_usage(&self.screen.ssao_resources.blur_v_params_buffer),
        );
        ledger.add_usage(
            GpuMemoryCategory::Postprocess,
            buffer_usage(&self.screen.ssao_compose.params_buffer),
        );
        ledger.add_usage(
            GpuMemoryCategory::Postprocess,
            buffer_usage(&self.screen.fxaa_pass.params_buffer),
        );

        ledger.add_usage(GpuMemoryCategory::Shadow, self.ctx.lighting.memory_usage());
        ledger.add_usage(
            GpuMemoryCategory::Shadow,
            self.lighting.directional_shadow.memory_usage(),
        );
        ledger.add_usage(
            GpuMemoryCategory::Shadow,
            self.lighting.atlas_ao.memory_usage(),
        );

        ledger.snapshot()
    }
}
