use super::math::hash_object_ids;
use super::state::*;
use crate::byte_units::bytes_to_mib;
use crate::map_contour::MapEntry;
use crate::memory::GpuMemoryUsage;
use crate::picking::pass::PickingParams;
use crate::picking::RepKind;
use crate::render_input::{RenderInput, RenderMapInput, RenderObjectInput, RenderStrokeInput};
use crate::representation_budget::{
    current_warning_keys, plan_rep_budget, RepBudgetDiagnostic, RepBudgetInput, RepBudgetRequest,
    RepBuildDecision, RepMemoryEstimate, RepQualityLevel,
};
use crate::representations::catalog::{self, RepCatalogEntry};
use crate::scene_store::SceneStoreCompactionStats;
use patinae_mol::{AtomIndex, DirtyFlags};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepSyncOutcome {
    Unchanged,
    Updated,
    Created,
}

impl RepSyncOutcome {
    fn created(self) -> bool {
        matches!(self, Self::Created)
    }
}

impl RenderState {
    /// Pull-based scene update. Rebuilds dirty representations and uploads
    /// material LUTs.
    pub fn sync(&mut self, input: &RenderInput) {
        self.sync_impl(input, None);
    }

    pub fn sync_with_timer<F>(&mut self, input: &RenderInput, mut now_ms: F)
    where
        F: FnMut() -> f64,
    {
        let timer: &mut dyn FnMut() -> f64 = &mut now_ms;
        self.sync_impl(input, Some(timer));
    }

    pub fn last_sync_timings(&self) -> RenderSyncTimings {
        self.last_sync_timings
    }

    fn sync_impl(&mut self, input: &RenderInput, mut now_ms: Option<&mut dyn FnMut() -> f64>) {
        #[cfg(feature = "stats")]
        {
            self.stats.frame_begin();

            self.stats.mark_sync_start();
        }

        let mut timings = RenderSyncTimings::default();
        let t0 = sync_now(&mut now_ms);

        // Track representation cache lifetime separately from the reps drawn
        // this frame. Surface/mesh/cartoon/ribbon can stay cached while
        // hidden because their shaders discard per-rep visibility, but
        // render passes must only iterate `draw_keep`.

        let mut keep: HashSet<(u32, RepKind)> = HashSet::with_capacity(self.scene.reps.len());
        let mut draw_keep: HashSet<(u32, RepKind)> = HashSet::with_capacity(self.scene.reps.len());
        let mut effective_dirty_by_object: HashMap<u32, DirtyFlags> =
            HashMap::with_capacity(input.objects.len());

        let object_hash = hash_object_ids(input.objects.iter().map(|o| o.object_id.0));

        let marker_dirty = object_hash != self.scene.marker_object_hash
            || input.objects.iter().any(|o| {
                !self.scene.scene_store.has_slot(o.object_id)
                    || o.dirty.intersects(
                        patinae_mol::DirtyFlags::SELECTION
                            | patinae_mol::DirtyFlags::HOVER
                            | patinae_mol::DirtyFlags::TOPOLOGY,
                    )
            });

        if marker_dirty {
            self.scene.has_any_marker = input.objects.iter().any(|o| o.has_markers);

            self.scene.marker_object_hash = object_hash;
            if self.screen.selection_overlay_enabled && self.screen.silhouette_params.is_none() {
                self.invalidate_overlay_id_cache();
            }
        }

        // Detect objects whose molecule content was swapped under the same

        // `ObjectId` (host re-loaded into the same slot — atom count

        // changed). Drop per-rep state for those objects so the reps

        // re-create their instance / indirect buffers against the new

        // atom count. SceneStore handles its own orphaning in `ensure_slot`.

        let mut evict_object_ids: Vec<u32> = Vec::new();

        for obj in input.objects {
            if let Some(existing) = self.scene.scene_store.slot(obj.object_id) {
                let new_atom_count = obj.molecule.atom_count() as u32;

                if existing.atom_count != new_atom_count {
                    evict_object_ids.push(obj.object_id.0);
                }
            }
        }

        if !evict_object_ids.is_empty() {
            self.scene
                .reps
                .retain(|(obj_id, _), _| !evict_object_ids.contains(obj_id));

            self.scene.scene_dirty = true;
        }

        // Marker LUT changes do not rebuild representation buffers or scene
        // ordering/bounds. The host still requests a redraw so the marking
        // overlay sees the updated LUT; geometry / visibility changes
        // additionally invalidate cached id textures below.
        let lod_dirty = self.scene.lod != input.lod;
        if lod_dirty {
            self.scene.lod = input.lod;
        }

        let mut scene_changed = !evict_object_ids.is_empty() || lod_dirty;
        let mut picking_cache_dirty = !evict_object_ids.is_empty();
        let mut draw_order_dirty = !evict_object_ids.is_empty();
        let mut map_order_dirty = false;
        let mut stroke_order_dirty = false;
        let mut bounds_dirty = !evict_object_ids.is_empty();

        let scene_affecting_dirty = patinae_mol::DirtyFlags::COORDS
            | patinae_mol::DirtyFlags::REPS
            | patinae_mol::DirtyFlags::VISIBILITY
            | patinae_mol::DirtyFlags::DRAW_MASK
            | patinae_mol::DirtyFlags::TOPOLOGY
            | patinae_mol::DirtyFlags::LOD;
        let draw_order_affecting_dirty = patinae_mol::DirtyFlags::REPS
            | patinae_mol::DirtyFlags::VISIBILITY
            | patinae_mol::DirtyFlags::DRAW_MASK
            | patinae_mol::DirtyFlags::TOPOLOGY
            | patinae_mol::DirtyFlags::LOD;
        let bounds_affecting_dirty = patinae_mol::DirtyFlags::COORDS
            | patinae_mol::DirtyFlags::REPS
            | patinae_mol::DirtyFlags::VISIBILITY
            | patinae_mol::DirtyFlags::TOPOLOGY;

        let mut object_offsets_changed = false;

        // Push every input object into the scene store first — every rep

        // reads its data from there.

        for obj in input.objects {
            let was_evicted = evict_object_ids.contains(&obj.object_id.0);

            let slot_was_new = !self.scene.scene_store.has_slot(obj.object_id);

            let effective_dirty = if was_evicted || slot_was_new {
                // Brand-new slot OR slot replaced under same id ⇒ rewrite

                // everything (atoms, coords, color, mask, marker, bonds, csr,

                // obj_table). The host's incoming `dirty` may underreport

                // (e.g. only COORDS) when the molecule content was swapped.

                patinae_mol::DirtyFlags::ALL
            } else {
                let mut dirty = obj.dirty;
                if lod_dirty {
                    dirty |= DirtyFlags::LOD;
                }
                dirty
            };
            effective_dirty_by_object.insert(obj.object_id.0, effective_dirty);

            if slot_was_new || effective_dirty.intersects(scene_affecting_dirty) {
                scene_changed = true;
                picking_cache_dirty = true;
            }

            if slot_was_new || was_evicted || effective_dirty.intersects(draw_order_affecting_dirty)
            {
                draw_order_dirty = true;
            }

            if slot_was_new || was_evicted || effective_dirty.intersects(bounds_affecting_dirty) {
                bounds_dirty = true;
            }

            if slot_was_new || was_evicted {
                object_offsets_changed = true;
            }

            self.scene.scene_store.sync_object(obj, effective_dirty);
        }
        if self
            .scene
            .scene_store
            .retain_objects(input.objects.iter().map(|obj| obj.object_id))
        {
            scene_changed = true;
            picking_cache_dirty = true;
            draw_order_dirty = true;
            bounds_dirty = true;
            object_offsets_changed = true;
        }
        let mut scene_store_compaction = if self
            .scene
            .scene_store
            .should_compact(self.memory.policy.scene_store.compaction)
        {
            self.scene.scene_store.compact()
        } else {
            Default::default()
        };
        if scene_store_compaction.ran {
            scene_changed = true;
            picking_cache_dirty = true;
            draw_order_dirty = true;
            bounds_dirty = true;
            object_offsets_changed = true;
            self.memory.rep_budget_request_cache.clear();
        }
        timings.scene_store_object_sync_ms = sync_elapsed(&mut now_ms, t0);

        let t0 = sync_now(&mut now_ms);
        let mut flush_stats = self.scene.scene_store.flush(
            &self.ctx.device,
            &self.ctx.queue,
            &self.scene.scene_layout,
            self.memory.policy.scene_store.growth,
        );
        scene_store_compaction.finish_after_flush(flush_stats.fragmentation.capacity_bytes);
        flush_stats.compaction = scene_store_compaction;
        timings.scene_store_flush_ms = sync_elapsed(&mut now_ms, t0);
        timings.marker_lut_upload_bytes = flush_stats.marker_lut_upload_bytes;
        timings.marker_lut_upload_ranges = flush_stats.marker_lut_upload_ranges;
        timings.marker_lut_reallocated = flush_stats.marker_lut_reallocated;
        timings.scene_store_fragmentation = flush_stats.fragmentation;
        timings.scene_store_compaction = flush_stats.compaction;
        if flush_stats.compaction.ran {
            log_scene_store_compaction(flush_stats.compaction, self.memory.policy.profile);
        }

        if object_offsets_changed {
            self.screen.marking_offsets_dirty = true;
        }
        if flush_stats.marker_lut_reallocated {
            self.screen.marking_bindings_dirty = true;
        }

        let t0 = sync_now(&mut now_ms);
        self.sync_selection_dots(input, &effective_dirty_by_object);
        self.refresh_marking_resources();
        timings.marking_resources_ms = sync_elapsed(&mut now_ms, t0);

        let t0 = sync_now(&mut now_ms);
        let rep_budget_diagnostics =
            self.plan_rep_budget_for_input(input, &effective_dirty_by_object);
        let budget_plans = rep_budget_plan_map(&rep_budget_diagnostics);
        self.update_rep_budget_diagnostics(rep_budget_diagnostics);

        for obj in input.objects {
            for rep in catalog::active_entries(obj.draw_reps) {
                let key = (obj.object_id.0, rep.kind);
                let budget_plan = budget_plans
                    .get(&key)
                    .copied()
                    .unwrap_or_else(RepBudgetPlan::build);
                if budget_plan.decision.is_skip() {
                    continue;
                }
                if self
                    .sync_rep(obj, input.settings, rep, lod_dirty, budget_plan)
                    .created()
                {
                    scene_changed = true;
                    picking_cache_dirty = true;
                    draw_order_dirty = true;
                    bounds_dirty = true;
                }
                keep.insert(key);
                draw_keep.insert(key);
            }
        }

        for obj in input.objects {
            let dirty = effective_dirty_by_object
                .get(&obj.object_id.0)
                .copied()
                .unwrap_or(obj.dirty);
            if !can_retain_hidden_rep_cache(dirty) {
                continue;
            }
            let hidden_cached_keys: Vec<(u32, RepKind)> = self
                .scene
                .reps
                .keys()
                .copied()
                .filter(|key| key.0 == obj.object_id.0)
                .filter(|key| hidden_rep_cacheable(key.1))
                .filter(|key| {
                    catalog::entry(key.1)
                        .is_some_and(|entry| obj.visible_reps.is_visible(entry.mask))
                })
                .filter(|key| !draw_keep.contains(key))
                .filter(|key| {
                    hidden_cache_allowed_by_budget(
                        *key,
                        &budget_plans,
                        self.memory.policy.reps.enabled,
                    )
                })
                .collect();
            keep.extend(hidden_cached_keys);
        }
        timings.rep_sync_ms = sync_elapsed(&mut now_ms, t0);

        // Drop reps for objects/representations no longer in the input.

        let t0 = sync_now(&mut now_ms);
        if rep_key_set_changed(
            self.scene.reps.keys().copied(),
            self.scene.reps.len(),
            &keep,
        ) {
            scene_changed = true;
            picking_cache_dirty = true;
            draw_order_dirty = true;
            bounds_dirty = true;

            self.scene.reps.retain(|k, _| keep.contains(k));
        }
        timings.order_bounds_ms += sync_elapsed(&mut now_ms, t0);

        let t0 = sync_now(&mut now_ms);
        let mut keep_maps: HashSet<u32> = HashSet::with_capacity(input.maps.len());
        for map in input.maps {
            keep_maps.insert(map.object_id.0);
            let entry_existed = self.scene.maps.contains_key(&map.object_id.0);
            if !entry_existed {
                let entry = MapEntry::new(
                    map,
                    &self.ctx.device,
                    &self.ctx.queue,
                    &self.geometry.map_params_layout,
                );
                self.scene.maps.insert(map.object_id.0, entry);
                scene_changed = true;
                map_order_dirty = true;
                bounds_dirty = true;
                continue;
            }
            let Some(entry) = self.scene.maps.get_mut(&map.object_id.0) else {
                continue;
            };
            if entry.sync(map, &self.ctx.device, &self.ctx.queue) {
                scene_changed = true;
                bounds_dirty = true;
            }
        }

        if self.scene.maps.len() != keep_maps.len() {
            scene_changed = true;
            map_order_dirty = true;
            bounds_dirty = true;
            self.scene
                .maps
                .retain(|object_id, _| keep_maps.contains(object_id));
        }

        if !map_order_dirty
            && !self
                .scene
                .map_draw_order
                .iter()
                .copied()
                .eq(input.maps.iter().map(|map| map.object_id.0))
        {
            scene_changed = true;
            map_order_dirty = true;
        }
        timings.map_sync_ms = sync_elapsed(&mut now_ms, t0);

        let t0 = sync_now(&mut now_ms);
        let stroke_bounds_hash = hash_stroke_bounds(input.strokes);
        if self.scene.stroke_bounds_hash != stroke_bounds_hash {
            self.scene.stroke_bounds_hash = stroke_bounds_hash;
            bounds_dirty = true;
            scene_changed = true;
        }
        let mut keep_strokes: HashSet<u32> = HashSet::with_capacity(input.strokes.len());
        for stroke in input
            .strokes
            .iter()
            .filter(|stroke| has_drawable_strokes(stroke))
        {
            keep_strokes.insert(stroke.object_id.0);
            let entry_existed = self.scene.strokes.contains_key(&stroke.object_id.0);
            if !entry_existed {
                let entry = crate::stroke::StrokeEntry::new(stroke, &self.ctx.device);
                self.scene.strokes.insert(stroke.object_id.0, entry);
                scene_changed = true;
                stroke_order_dirty = true;
                bounds_dirty = true;
                continue;
            }
            let Some(entry) = self.scene.strokes.get_mut(&stroke.object_id.0) else {
                continue;
            };
            if entry.sync(stroke, &self.ctx.device, &self.ctx.queue) {
                scene_changed = true;
                bounds_dirty = true;
            }
        }
        if self.scene.strokes.len() != keep_strokes.len() {
            self.scene
                .strokes
                .retain(|object_id, _| keep_strokes.contains(object_id));
            scene_changed = true;
            stroke_order_dirty = true;
            bounds_dirty = true;
        }
        if !stroke_order_dirty
            && !self.scene.stroke_draw_order.iter().copied().eq(input
                .strokes
                .iter()
                .filter(|stroke| has_drawable_strokes(stroke))
                .map(|stroke| stroke.object_id.0))
        {
            scene_changed = true;
            stroke_order_dirty = true;
        }
        timings.stroke_sync_ms = sync_elapsed(&mut now_ms, t0);

        // Rebuild stable draw order. Sort by `RepKind` discriminant first so

        // every pass groups pipeline switches; tie-break by `object_id` for

        // deterministic ordering across runs.

        let t0 = sync_now(&mut now_ms);
        if draw_order_dirty {
            self.scene.draw_order.clear();

            self.scene.draw_order.extend(draw_keep.iter().copied());

            self.scene
                .draw_order
                .sort_by_key(|(obj_id, kind)| (*kind as u32, *obj_id));
        }

        if map_order_dirty {
            self.scene.map_draw_order.clear();
            self.scene
                .map_draw_order
                .extend(input.maps.iter().map(|map| map.object_id.0));
        }

        if stroke_order_dirty {
            self.scene.stroke_draw_order.clear();
            self.scene.stroke_draw_order.extend(
                input
                    .strokes
                    .iter()
                    .filter(|stroke| has_drawable_strokes(stroke))
                    .map(|stroke| stroke.object_id.0),
            );
        }

        if bounds_dirty || self.scene.scene_bounds.is_none() {
            self.scene.scene_bounds =
                compute_scene_bounds(input.objects, input.maps, input.strokes);
        }

        if self.sync_manual_picking(input, draw_keep.len()) {
            picking_cache_dirty = true;
        }
        timings.order_bounds_ms += sync_elapsed(&mut now_ms, t0);

        // Dispatch GPU instance-build compute for SceneStore-resident

        // reps (sphere today; stick/line/dot/ellipsoid follow). One

        // command encoder per sync; submitted iff at least one rep

        // recorded work.

        let t0 = sync_now(&mut now_ms);
        let compute_dispatched = self.dispatch_pending_compute_builds();
        timings.compute_dispatch_ms = sync_elapsed(&mut now_ms, t0);
        if compute_dispatched {
            scene_changed = true;
            picking_cache_dirty = true;
        }

        if scene_changed {
            self.scene.scene_dirty = true;
            self.invalidate_cull_cache();
        }

        if picking_cache_dirty {
            self.invalidate_picking_cache();
            self.invalidate_overlay_id_cache();
        }

        #[cfg(feature = "stats")]
        self.stats.mark_sync_end();

        self.last_sync_timings = timings;
    }

    fn sync_rep(
        &mut self,

        obj: &RenderObjectInput<'_>,

        settings: &patinae_settings::ResolvedSettings,

        rep_catalog: &'static RepCatalogEntry,

        lod_dirty: bool,

        budget_plan: RepBudgetPlan,
    ) -> RepSyncOutcome {
        let kind = rep_catalog.kind;
        let key = (obj.object_id.0, kind);

        let entry_existed = self.scene.reps.contains_key(&key);
        let budget_dirty = if entry_existed {
            self.scene.reps.get_mut(&key).is_some_and(|entry| {
                entry
                    .rep
                    .apply_budget_decision(budget_plan.decision, budget_plan.quality)
            })
        } else {
            false
        };

        // Fast path: entry already exists and the host has no pending dirty
        // bits for this object. Skip the entire build — no atom iteration,
        // no `write_buffer`. Dominant cost on assemblies with hundreds of
        // chain copies; rotating the camera redraws the same geometry
        // without any per-rep CPU work.

        if entry_existed && obj.dirty.is_empty() && !lod_dirty && !budget_dirty {
            return RepSyncOutcome::Unchanged;
        }

        let marker_or_draw_only = obj
            .dirty
            .difference(
                patinae_mol::DirtyFlags::SELECTION
                    | patinae_mol::DirtyFlags::HOVER
                    | patinae_mol::DirtyFlags::DRAW_MASK,
            )
            .is_empty();

        if entry_existed && marker_or_draw_only && !lod_dirty && !budget_dirty {
            return RepSyncOutcome::Unchanged;
        }

        let entry = self.scene.reps.entry(key).or_insert_with(|| {
            let rep = rep_catalog.construct(&self.ctx.device);

            let picking = self.picking.id_pass.as_ref().map(|pp| {
                let params = PickingParams::new(kind, obj.object_id);

                let (buffer, bind_group) = pp.make_params(&self.ctx.device, params);

                RepPickingState {
                    _buffer: buffer,

                    bind_group,
                }
            });

            RepEntry {
                rep,

                picking,

                draw_phase: crate::representations::DrawPhase::Opaque,
            }
        });

        if !entry_existed {
            entry
                .rep
                .apply_budget_decision(budget_plan.decision, budget_plan.quality);
        }

        // For a freshly-created rep the host's dirty mask may be empty

        // (e.g. visible_reps just toggled SPHERES on without touching atoms).

        // Force a full rebuild on first sight so the rep has buffers.

        let effective_dirty = if entry_existed {
            let mut dirty = obj.dirty;
            if lod_dirty {
                dirty |= patinae_mol::DirtyFlags::LOD;
            }
            dirty
        } else {
            patinae_mol::DirtyFlags::ALL
        };

        entry.rep.build(
            obj,
            settings,
            effective_dirty,
            &self.ctx.device,
            &self.ctx.queue,
        );

        entry.draw_phase = entry.rep.draw_phase();
        if entry_existed {
            RepSyncOutcome::Updated
        } else {
            RepSyncOutcome::Created
        }
    }

    fn plan_rep_budget_for_input(
        &mut self,
        input: &RenderInput,
        effective_dirty_by_object: &HashMap<u32, DirtyFlags>,
    ) -> Vec<RepBudgetDiagnostic> {
        let active_rep_count = input
            .objects
            .iter()
            .map(|obj| catalog::active_entries(obj.draw_reps).count())
            .sum();
        let mut requests = Vec::with_capacity(active_rep_count);
        let estimate_allocations = self.memory.policy.reps.enabled;
        if !estimate_allocations {
            self.memory.rep_budget_request_cache.clear();
        }
        let mut active_budget_keys = HashSet::with_capacity(active_rep_count);
        for obj in input.objects {
            for rep in catalog::active_entries(obj.draw_reps) {
                let key = (obj.object_id.0, rep.kind);
                if !estimate_allocations {
                    requests.push(cheap_budget_request(obj, rep));
                    continue;
                }

                active_budget_keys.insert(key);
                let dirty = effective_dirty_by_object
                    .get(&obj.object_id.0)
                    .copied()
                    .unwrap_or(obj.dirty);
                if should_reuse_budget_request(rep, dirty) {
                    if let Some(cached) = self.memory.rep_budget_request_cache.get(&key) {
                        requests.push(cached.clone());
                        continue;
                    }
                }

                let request = rep.budget_request(obj, input.settings);
                self.memory
                    .rep_budget_request_cache
                    .insert(key, request.clone());
                requests.push(request);
            }
        }
        if estimate_allocations {
            self.memory
                .rep_budget_request_cache
                .retain(|key, _| active_budget_keys.contains(key));
        }
        let snapshot = self.memory_snapshot();
        let fixed_reserved_bytes = snapshot
            .total_capacity_bytes()
            .saturating_sub(replaceable_rep_capacity_bytes(&self.scene));
        plan_rep_budget(RepBudgetInput {
            policy: self.memory.policy,
            fixed_reserved_bytes,
            device_limits: &self.ctx.device.limits(),
            requests: &requests,
            active_rep_count,
        })
    }

    fn update_rep_budget_diagnostics(&mut self, diagnostics: Vec<RepBudgetDiagnostic>) {
        let current_warning_keys = current_warning_keys(&diagnostics);
        for diagnostic in &diagnostics {
            let Some(key) = diagnostic.warning_key() else {
                continue;
            };
            if !self.memory.warned_rep_budget.contains(&key) {
                let message = rep_budget_warning(*diagnostic, self.memory.policy.profile);
                log::warn!("patinae-render: {message}");
                self.memory.pending_warnings.push(message);
            }
        }
        self.memory
            .warned_rep_budget
            .retain(|key| current_warning_keys.contains(key));
        self.memory.warned_rep_budget.extend(current_warning_keys);
        self.memory.rep_budget_diagnostics = diagnostics;
    }
}

fn should_reuse_budget_request(rep: &'static RepCatalogEntry, dirty: DirtyFlags) -> bool {
    dirty.is_empty() || !rep.budget_estimate_invalidated_by(dirty)
}

fn cheap_budget_request(
    obj: &RenderObjectInput<'_>,
    rep: &'static RepCatalogEntry,
) -> RepBudgetRequest {
    RepBudgetRequest::new(
        obj.object_id,
        rep.kind,
        vec![RepMemoryEstimate {
            required_bytes: 0,
            scratch_bytes: 0,
            capacity_bytes: 0,
            quality: RepQualityLevel::Full,
            can_chunk: false,
            can_skip: true,
        }],
    )
}

#[derive(Debug, Clone, Copy)]
struct RepBudgetPlan {
    decision: RepBuildDecision,
    quality: RepQualityLevel,
}

impl RepBudgetPlan {
    const fn build() -> Self {
        Self {
            decision: RepBuildDecision::Build,
            quality: RepQualityLevel::Full,
        }
    }

    const fn from_diagnostic(diagnostic: RepBudgetDiagnostic) -> Self {
        Self {
            decision: diagnostic.decision,
            quality: diagnostic.estimate.quality,
        }
    }
}

fn rep_budget_plan_map(
    diagnostics: &[RepBudgetDiagnostic],
) -> HashMap<(u32, RepKind), RepBudgetPlan> {
    let mut decisions = HashMap::with_capacity(diagnostics.len());
    decisions.extend(diagnostics.iter().map(|diagnostic| {
        (
            (diagnostic.object_id.0, diagnostic.kind),
            RepBudgetPlan::from_diagnostic(*diagnostic),
        )
    }));
    decisions
}

fn hidden_cache_allowed_by_budget(
    key: (u32, RepKind),
    decisions: &HashMap<(u32, RepKind), RepBudgetPlan>,
    budget_enforced: bool,
) -> bool {
    if !budget_enforced {
        return true;
    }
    decisions
        .get(&key)
        .is_some_and(|plan| !plan.decision.is_skip())
}

fn replaceable_rep_capacity_bytes(scene: &SceneRuntime) -> u64 {
    scene.reps.values().fold(0_u64, |bytes, entry| {
        let rep = entry.rep.memory_usage();
        let picking = entry
            .picking
            .as_ref()
            .map(|picking| GpuMemoryUsage::allocation(picking._buffer.size()))
            .unwrap_or_default();
        bytes
            .saturating_add(rep.capacity_bytes)
            .saturating_add(picking.capacity_bytes)
    })
}

fn rep_budget_warning(
    diagnostic: RepBudgetDiagnostic,
    profile: crate::RenderMemoryProfile,
) -> String {
    match diagnostic.decision {
        RepBuildDecision::Downgrade => format!(
            "{:?} for object {} downgraded by render memory profile {} to {:?} ({:.2} MiB estimated).",
            diagnostic.kind,
            diagnostic.object_id.0,
            profile,
            diagnostic.estimate.quality,
            bytes_to_mib(diagnostic.estimate.reserved_bytes())
        ),
        RepBuildDecision::Chunk { max_chunk_bytes } => format!(
            "{:?} for object {} chunked by render memory profile {} to {:.2} MiB chunks ({:.2} MiB estimated).",
            diagnostic.kind,
            diagnostic.object_id.0,
            profile,
            bytes_to_mib(max_chunk_bytes),
            bytes_to_mib(diagnostic.estimate.reserved_bytes())
        ),
        RepBuildDecision::Skip { reason } => format!(
            "{:?} for object {} skipped by render memory profile {}: {:?} ({:.2} MiB estimated).",
            diagnostic.kind,
            diagnostic.object_id.0,
            profile,
            reason,
            bytes_to_mib(diagnostic.estimate.reserved_bytes())
        ),
        RepBuildDecision::Build => String::new(),
    }
}

fn log_scene_store_compaction(
    compaction: SceneStoreCompactionStats,
    profile: crate::RenderMemoryProfile,
) {
    let largest = compaction
        .largest_orphaned_buffer
        .map(|kind| kind.label())
        .unwrap_or("none");
    log::info!(
        "patinae-render: [gpu-mem] scene_store compacted profile={} reclaimed={:.2} MiB capacity_before={:.2} MiB capacity_after={:.2} MiB orphaned_before={:.2} MiB moved_objects={} largest_orphaned={}",
        profile,
        bytes_to_mib(compaction.reclaimed_bytes),
        bytes_to_mib(compaction.capacity_before_bytes),
        bytes_to_mib(compaction.capacity_after_bytes),
        bytes_to_mib(compaction.orphaned_before_bytes),
        compaction.moved_objects,
        largest
    );
}

fn rep_key_set_changed(
    mut current_keys: impl Iterator<Item = (u32, RepKind)>,
    current_len: usize,
    keep: &HashSet<(u32, RepKind)>,
) -> bool {
    current_len != keep.len() || current_keys.any(|key| !keep.contains(&key))
}

fn hidden_rep_cacheable(kind: RepKind) -> bool {
    matches!(
        kind,
        RepKind::Surface | RepKind::Mesh | RepKind::Cartoon | RepKind::Ribbon
    )
}

fn has_drawable_strokes(input: &RenderStrokeInput<'_>) -> bool {
    !input.segments.is_empty()
}

fn hash_stroke_bounds(strokes: &[RenderStrokeInput<'_>]) -> u64 {
    let mut hasher = DefaultHasher::new();
    strokes.len().hash(&mut hasher);
    for stroke in strokes {
        stroke.object_id.0.hash(&mut hasher);
        stroke.geometry_revision.hash(&mut hasher);
        match stroke.bounds {
            Some((min, max)) => {
                1_u8.hash(&mut hasher);
                for value in min.into_iter().chain(max) {
                    value.to_bits().hash(&mut hasher);
                }
            }
            None => 0_u8.hash(&mut hasher),
        }
    }
    hasher.finish()
}

fn can_retain_hidden_rep_cache(dirty: DirtyFlags) -> bool {
    const RETAINABLE_WHILE_HIDDEN: DirtyFlags = DirtyFlags::COLOR
        .union(DirtyFlags::VISIBILITY)
        .union(DirtyFlags::DRAW_MASK)
        .union(DirtyFlags::SELECTION)
        .union(DirtyFlags::HOVER);
    dirty.is_empty() || dirty.difference(RETAINABLE_WHILE_HIDDEN).is_empty()
}

fn compute_scene_bounds(
    objects: &[RenderObjectInput<'_>],
    maps: &[RenderMapInput<'_>],
    strokes: &[RenderStrokeInput<'_>],
) -> Option<SceneBounds> {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut any = false;
    for obj in objects {
        if obj.draw_reps.0 == 0 {
            continue;
        }
        for atom_idx in 0..obj.molecule.atom_count() {
            let idx = AtomIndex(atom_idx as u32);
            let Some(atom) = obj.molecule.get_atom(idx) else {
                continue;
            };
            if (atom.repr.visible_reps.0 & obj.draw_reps.0) == 0 {
                continue;
            }
            let Some(pos) = obj.coord_set.get_atom_coord(idx) else {
                continue;
            };
            let pad = atom.effective_vdw().max(0.1);
            let p = transform_point(&obj.transform, [pos.x, pos.y, pos.z]);
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis] - pad);
                max[axis] = max[axis].max(p[axis] + pad);
            }
            any = true;
        }
    }
    for map in maps {
        let vd = map.grid.vertex_dims();
        let min_grid = map.grid.origin;
        let max_grid = [
            min_grid[0] + (vd[0] - 1) as f32 * map.grid.spacing[0],
            min_grid[1] + (vd[1] - 1) as f32 * map.grid.spacing[1],
            min_grid[2] + (vd[2] - 1) as f32 * map.grid.spacing[2],
        ];
        for corner in bbox_corners(min_grid, max_grid) {
            let p = transform_point(&map.transform, corner);
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
            any = true;
        }
    }
    for stroke in strokes {
        let Some((stroke_min, stroke_max)) = stroke.bounds else {
            continue;
        };
        for axis in 0..3 {
            min[axis] = min[axis].min(stroke_min[axis]);
            max[axis] = max[axis].max(stroke_max[axis]);
        }
        any = true;
    }
    if !any {
        return None;
    }
    let center = [
        0.5 * (min[0] + max[0]),
        0.5 * (min[1] + max[1]),
        0.5 * (min[2] + max[2]),
    ];
    let extent = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let radius = (0.5
        * (extent[0] * extent[0] + extent[1] * extent[1] + extent[2] * extent[2]).sqrt())
    .max(1.0);
    Some(SceneBounds { center, radius })
}

fn bbox_corners(min: [f32; 3], max: [f32; 3]) -> [[f32; 3]; 8] {
    [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [min[0], max[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [min[0], max[1], max[2]],
        [max[0], max[1], max[2]],
    ]
}

fn transform_point(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
    ]
}

fn sync_now(now_ms: &mut Option<&mut dyn FnMut() -> f64>) -> f64 {
    match now_ms.as_deref_mut() {
        Some(now) => now(),
        None => 0.0,
    }
}

fn sync_elapsed(now_ms: &mut Option<&mut dyn FnMut() -> f64>, start_ms: f64) -> f32 {
    if now_ms.is_none() {
        0.0
    } else {
        (sync_now(now_ms) - start_ms).max(0.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rep_key_set_changed_detects_same_length_key_replacement() {
        let keep = HashSet::from([(2, RepKind::Sphere)]);

        assert!(rep_key_set_changed(
            [(1, RepKind::Sphere)].into_iter(),
            1,
            &keep
        ));
    }

    #[test]
    fn rep_key_set_changed_accepts_matching_keys() {
        let keep = HashSet::from([(1, RepKind::Sphere), (1, RepKind::Stick)]);

        assert!(!rep_key_set_changed(
            [(1, RepKind::Sphere), (1, RepKind::Stick)].into_iter(),
            2,
            &keep
        ));
    }

    #[test]
    fn hidden_cache_only_keeps_shader_gated_heavy_reps() {
        assert!(hidden_rep_cacheable(RepKind::Surface));
        assert!(hidden_rep_cacheable(RepKind::Mesh));
        assert!(hidden_rep_cacheable(RepKind::Cartoon));
        assert!(hidden_rep_cacheable(RepKind::Ribbon));
        assert!(!hidden_rep_cacheable(RepKind::Sphere));
        assert!(!hidden_rep_cacheable(RepKind::Stick));
    }

    #[test]
    fn hidden_cache_retention_is_lut_only() {
        assert!(can_retain_hidden_rep_cache(DirtyFlags::empty()));
        assert!(can_retain_hidden_rep_cache(DirtyFlags::VISIBILITY));
        assert!(can_retain_hidden_rep_cache(DirtyFlags::DRAW_MASK));
        assert!(can_retain_hidden_rep_cache(
            DirtyFlags::COLOR | DirtyFlags::SELECTION | DirtyFlags::DRAW_MASK
        ));
        assert!(!can_retain_hidden_rep_cache(DirtyFlags::TRANSPARENCY));
        assert!(!can_retain_hidden_rep_cache(DirtyFlags::REPS));
        assert!(!can_retain_hidden_rep_cache(DirtyFlags::COORDS));
        assert!(!can_retain_hidden_rep_cache(DirtyFlags::LOD));
        assert!(!can_retain_hidden_rep_cache(DirtyFlags::TOPOLOGY));
    }

    #[test]
    fn budget_request_cache_reuses_clean_and_lut_only_dirty() {
        let cartoon = catalog::entry(RepKind::Cartoon).expect("cartoon catalog entry");

        assert!(should_reuse_budget_request(cartoon, DirtyFlags::empty()));
        assert!(should_reuse_budget_request(cartoon, DirtyFlags::COLOR));
        assert!(should_reuse_budget_request(cartoon, DirtyFlags::SELECTION));
        assert!(should_reuse_budget_request(cartoon, DirtyFlags::HOVER));
        assert!(should_reuse_budget_request(
            cartoon,
            DirtyFlags::TRANSPARENCY
        ));
    }

    #[test]
    fn budget_request_cache_reuses_geometry_reps_on_coords_dirty() {
        let cartoon = catalog::entry(RepKind::Cartoon).expect("cartoon catalog entry");
        let surface = catalog::entry(RepKind::Surface).expect("surface catalog entry");

        assert!(should_reuse_budget_request(cartoon, DirtyFlags::COORDS));
        assert!(should_reuse_budget_request(surface, DirtyFlags::COORDS));
    }

    #[test]
    fn budget_request_cache_reuses_count_reps_on_coords_dirty() {
        let sphere = catalog::entry(RepKind::Sphere).expect("sphere catalog entry");
        let stick = catalog::entry(RepKind::Stick).expect("stick catalog entry");
        let ellipsoid = catalog::entry(RepKind::Ellipsoid).expect("ellipsoid catalog entry");

        assert!(should_reuse_budget_request(sphere, DirtyFlags::COORDS));
        assert!(should_reuse_budget_request(stick, DirtyFlags::COORDS));
        assert!(should_reuse_budget_request(ellipsoid, DirtyFlags::COORDS));
    }

    #[test]
    fn budget_request_cache_recomputes_relevant_shape_dirty() {
        let cartoon = catalog::entry(RepKind::Cartoon).expect("cartoon catalog entry");
        let sphere = catalog::entry(RepKind::Sphere).expect("sphere catalog entry");

        for dirty in [
            DirtyFlags::TOPOLOGY,
            DirtyFlags::REPS,
            DirtyFlags::VISIBILITY,
            DirtyFlags::LOD,
        ] {
            assert!(!should_reuse_budget_request(cartoon, dirty));
            assert!(!should_reuse_budget_request(sphere, dirty));
        }
    }

    #[test]
    fn scene_bounds_include_stroke_only_payloads() {
        let input = RenderStrokeInput {
            object_id: crate::ObjectId(1),
            segments: &[],
            bounds: Some(([-2.0, -1.0, 0.0], [4.0, 3.0, 2.0])),
            geometry_revision: 1,
            material_revision: 1,
        };

        assert!(!has_drawable_strokes(&input));
        let bounds = compute_scene_bounds(&[], &[], &[input]).unwrap();
        assert_eq!(bounds.center, [1.0, 1.0, 1.0]);
        assert!((bounds.radius - 3.741_657_5).abs() < 1.0e-6);
    }

    #[test]
    fn label_only_bounds_have_an_independent_scene_fingerprint() {
        let first = RenderStrokeInput {
            object_id: crate::ObjectId(1),
            segments: &[],
            bounds: Some(([1.0, 2.0, 3.0], [1.0, 2.0, 3.0])),
            geometry_revision: 7,
            material_revision: 9,
        };
        let moved = RenderStrokeInput {
            object_id: crate::ObjectId(1),
            segments: &[],
            bounds: Some(([4.0, 5.0, 6.0], [4.0, 5.0, 6.0])),
            geometry_revision: 8,
            material_revision: 9,
        };

        assert_ne!(hash_stroke_bounds(&[first]), hash_stroke_bounds(&[moved]));
    }
}
