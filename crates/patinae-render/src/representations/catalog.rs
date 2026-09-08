//! Central registry for built-in render representations.
//!
//! The renderer does not expose a custom-representation extension point. This
//! table is the crate-local source of truth for built-in representation
//! construction and per-pass pipeline routing.

use patinae_mol::{DirtyFlags, RepMask};

use crate::compute::cull::CullPipeline;
use crate::picking::pass::PickingPass;
use crate::picking::RepKind;
use crate::render_input::RenderObjectInput;
use crate::render_state::state::GeometryRuntime;
use crate::representation_budget::{RepBudgetRequest, RepMemoryEstimate, RepQualityLevel};
use crate::representations::cartoon::CartoonRep;
use crate::representations::dot::DotRep;
use crate::representations::ellipsoid::EllipsoidRep;
use crate::representations::line::LineRep;
use crate::representations::sphere::SphereRep;
use crate::representations::stick::StickRep;
use crate::representations::surface::SurfaceRep;
use crate::representations::{DrawPhase, Representation};

type RepConstructor = fn(&wgpu::Device) -> Box<dyn Representation>;
type RepEstimator =
    fn(&RenderObjectInput<'_>, &patinae_settings::ResolvedSettings) -> Vec<RepMemoryEstimate>;

pub(crate) struct RepCatalogEntry {
    pub(crate) kind: RepKind,
    pub(crate) mask: RepMask,
    constructor: RepConstructor,
    estimator: RepEstimator,
    budget_invalidating_dirty: DirtyFlags,
    pub(crate) cullable: bool,
}

impl RepCatalogEntry {
    pub(crate) fn construct(&self, device: &wgpu::Device) -> Box<dyn Representation> {
        (self.constructor)(device)
    }

    pub(crate) fn budget_request(
        &self,
        input: &RenderObjectInput<'_>,
        settings: &patinae_settings::ResolvedSettings,
    ) -> RepBudgetRequest {
        let mut estimates = (self.estimator)(input, settings);
        if estimates.is_empty() {
            estimates.push(RepMemoryEstimate {
                required_bytes: 0,
                scratch_bytes: 0,
                capacity_bytes: 0,
                quality: RepQualityLevel::Full,
                can_chunk: false,
                can_skip: true,
            });
        }
        RepBudgetRequest::new(input.object_id, self.kind, estimates)
    }

    pub(crate) fn budget_estimate_invalidated_by(&self, dirty: DirtyFlags) -> bool {
        dirty.intersects(self.budget_invalidating_dirty)
    }

    pub(crate) fn color_pipeline<'a>(
        &self,
        geometry: &'a GeometryRuntime,
        phase: DrawPhase,
    ) -> Option<&'a wgpu::RenderPipeline> {
        match phase {
            DrawPhase::Opaque => self.opaque_pipeline(geometry),
            DrawPhase::Wboit => self.wboit_pipeline(geometry),
            DrawPhase::FastOverlay => self.fast_overlay_pipeline(geometry),
        }
    }

    pub(crate) fn picking_pipeline<'a>(
        &self,
        id_pass: &'a PickingPass,
    ) -> Option<&'a wgpu::RenderPipeline> {
        match self.kind {
            RepKind::Sphere => Some(&id_pass.sphere_pipeline),
            RepKind::Stick => Some(&id_pass.stick_pipeline),
            RepKind::Line => Some(&id_pass.line_pipeline),
            RepKind::Dot => Some(&id_pass.dot_pipeline),
            RepKind::Mesh => Some(&id_pass.std_vertex_line_pipeline),
            RepKind::Surface | RepKind::Cartoon | RepKind::Ribbon => {
                Some(&id_pass.std_vertex_pipeline)
            }
            RepKind::Ellipsoid => Some(&id_pass.ellipsoid_pipeline),
            _ => None,
        }
    }

    pub(crate) fn shadow_pipeline<'a>(
        &self,
        geometry: &'a GeometryRuntime,
    ) -> Option<&'a wgpu::RenderPipeline> {
        match self.kind {
            RepKind::Sphere => Some(&geometry.depth_prepass.sphere),
            RepKind::Stick => Some(&geometry.depth_prepass.stick),
            RepKind::Line => Some(&geometry.depth_prepass.line),
            RepKind::Dot => Some(&geometry.depth_prepass.dot),
            RepKind::Mesh => Some(&geometry.depth_prepass.mesh),
            RepKind::Surface => Some(&geometry.depth_prepass.surface),
            RepKind::Cartoon | RepKind::Ribbon => Some(&geometry.depth_prepass.cartoon),
            RepKind::Ellipsoid => Some(&geometry.depth_prepass.ellipsoid),
            _ => None,
        }
    }

    pub(crate) fn cull_pipeline<'a>(
        &self,
        cull: &'a CullPipeline,
    ) -> Option<&'a wgpu::ComputePipeline> {
        match self.kind {
            RepKind::Sphere => Some(&cull.pipeline_sphere),
            RepKind::Stick => Some(&cull.pipeline_stick),
            RepKind::Line => Some(&cull.pipeline_line),
            RepKind::Dot => Some(&cull.pipeline_dot),
            RepKind::Ellipsoid => Some(&cull.pipeline_ellipsoid),
            _ => None,
        }
    }

    pub(crate) fn cull_label(&self) -> &'static str {
        match self.kind {
            RepKind::Sphere => "patinae.cull.sphere",
            RepKind::Stick => "patinae.cull.stick",
            RepKind::Line => "patinae.cull.line",
            RepKind::Dot => "patinae.cull.dot",
            RepKind::Ellipsoid => "patinae.cull.ellipsoid",
            _ => "patinae.cull.unknown",
        }
    }

    fn opaque_pipeline<'a>(
        &self,
        geometry: &'a GeometryRuntime,
    ) -> Option<&'a wgpu::RenderPipeline> {
        match self.kind {
            RepKind::Sphere => Some(&geometry.sphere_pipeline.pipeline_opaque),
            RepKind::Stick => Some(&geometry.stick_pipeline.pipeline_opaque),
            RepKind::Line => Some(&geometry.line_pipeline.pipeline_opaque),
            RepKind::Dot => Some(&geometry.dot_pipeline.pipeline_opaque),
            RepKind::Mesh => Some(&geometry.mesh_pipeline.pipeline_opaque),
            RepKind::Surface => Some(&geometry.surface_pipeline.pipeline_opaque),
            RepKind::Cartoon | RepKind::Ribbon => Some(&geometry.cartoon_pipeline.pipeline_opaque),
            RepKind::Ellipsoid => Some(&geometry.ellipsoid_pipeline.pipeline_opaque),
            _ => None,
        }
    }

    fn wboit_pipeline<'a>(
        &self,
        geometry: &'a GeometryRuntime,
    ) -> Option<&'a wgpu::RenderPipeline> {
        match self.kind {
            RepKind::Sphere => Some(&geometry.sphere_pipeline.pipeline),
            RepKind::Stick => Some(&geometry.stick_pipeline.pipeline),
            RepKind::Line => Some(&geometry.line_pipeline.pipeline),
            RepKind::Dot => Some(&geometry.dot_pipeline.pipeline),
            RepKind::Mesh => Some(&geometry.mesh_pipeline.pipeline),
            RepKind::Surface => Some(&geometry.surface_pipeline.pipeline),
            RepKind::Cartoon | RepKind::Ribbon => Some(&geometry.cartoon_pipeline.pipeline),
            RepKind::Ellipsoid => Some(&geometry.ellipsoid_pipeline.pipeline),
            _ => None,
        }
    }

    fn fast_overlay_pipeline<'a>(
        &self,
        geometry: &'a GeometryRuntime,
    ) -> Option<&'a wgpu::RenderPipeline> {
        match self.kind {
            RepKind::Dot => Some(&geometry.dot_pipeline.pipeline_fast_overlay),
            RepKind::Mesh => Some(&geometry.mesh_pipeline.pipeline_fast_overlay),
            _ => None,
        }
    }
}

pub(crate) fn active_entries(
    visible_reps: RepMask,
) -> impl Iterator<Item = &'static RepCatalogEntry> {
    REPS.iter()
        .filter(move |entry| visible_reps.is_visible(entry.mask))
}

pub(crate) fn entry(kind: RepKind) -> Option<&'static RepCatalogEntry> {
    REPS.iter().find(|entry| entry.kind == kind)
}

fn sphere(device: &wgpu::Device) -> Box<dyn Representation> {
    Box::new(SphereRep::new(device))
}

fn estimate_sphere(
    input: &RenderObjectInput<'_>,
    _settings: &patinae_settings::ResolvedSettings,
) -> Vec<RepMemoryEstimate> {
    crate::representations::sphere::budget_estimates(input)
}

fn stick(device: &wgpu::Device) -> Box<dyn Representation> {
    Box::new(StickRep::new(device))
}

fn estimate_stick(
    input: &RenderObjectInput<'_>,
    settings: &patinae_settings::ResolvedSettings,
) -> Vec<RepMemoryEstimate> {
    crate::representations::stick::budget_estimates(input, settings)
}

fn line(device: &wgpu::Device) -> Box<dyn Representation> {
    Box::new(LineRep::new(device))
}

fn estimate_line(
    input: &RenderObjectInput<'_>,
    _settings: &patinae_settings::ResolvedSettings,
) -> Vec<RepMemoryEstimate> {
    crate::representations::line::budget_estimates(input)
}

fn dot(device: &wgpu::Device) -> Box<dyn Representation> {
    Box::new(DotRep::new(device))
}

fn estimate_dot(
    input: &RenderObjectInput<'_>,
    _settings: &patinae_settings::ResolvedSettings,
) -> Vec<RepMemoryEstimate> {
    crate::representations::dot::budget_estimates(input)
}

fn surface(device: &wgpu::Device) -> Box<dyn Representation> {
    Box::new(SurfaceRep::new(device))
}

fn estimate_surface(
    input: &RenderObjectInput<'_>,
    settings: &patinae_settings::ResolvedSettings,
) -> Vec<RepMemoryEstimate> {
    crate::representations::surface::budget_estimates(
        input,
        settings,
        crate::representations::surface::SurfaceMode::Surface,
    )
}

fn mesh(device: &wgpu::Device) -> Box<dyn Representation> {
    Box::new(SurfaceRep::new_mesh(device))
}

fn estimate_mesh(
    input: &RenderObjectInput<'_>,
    settings: &patinae_settings::ResolvedSettings,
) -> Vec<RepMemoryEstimate> {
    crate::representations::surface::budget_estimates(
        input,
        settings,
        crate::representations::surface::SurfaceMode::Mesh,
    )
}

fn cartoon(device: &wgpu::Device) -> Box<dyn Representation> {
    Box::new(CartoonRep::new(device))
}

fn estimate_cartoon(
    input: &RenderObjectInput<'_>,
    settings: &patinae_settings::ResolvedSettings,
) -> Vec<RepMemoryEstimate> {
    crate::representations::cartoon::budget_estimates(
        input,
        settings,
        crate::representations::cartoon::CartoonMode::Cartoon,
    )
}

fn ribbon(device: &wgpu::Device) -> Box<dyn Representation> {
    Box::new(CartoonRep::new_ribbon(device))
}

fn estimate_ribbon(
    input: &RenderObjectInput<'_>,
    settings: &patinae_settings::ResolvedSettings,
) -> Vec<RepMemoryEstimate> {
    crate::representations::cartoon::budget_estimates(
        input,
        settings,
        crate::representations::cartoon::CartoonMode::Ribbon,
    )
}

fn ellipsoid(device: &wgpu::Device) -> Box<dyn Representation> {
    Box::new(EllipsoidRep::new(device))
}

fn estimate_ellipsoid(
    input: &RenderObjectInput<'_>,
    _settings: &patinae_settings::ResolvedSettings,
) -> Vec<RepMemoryEstimate> {
    crate::representations::ellipsoid::budget_estimates(input)
}

// Every built-in rep budgets against *counts* (atom/partition topology,
// enabled reps, visibility, LOD) rather than coordinates. Surface/Mesh/
// Cartoon/Ribbon used to add COORDS here (`GEOMETRY_BUDGET_DIRTY`) because
// their estimators re-ran the full CPU geometry pipeline; the estimators are
// now cheap count-based upper bounds, so COORDS no longer invalidates the
// budget cache. This lets the budget request cache hit during coordinate-only
// reloads instead of re-estimating every geometry rep each frame.
const COUNT_BUDGET_DIRTY: DirtyFlags = DirtyFlags::TOPOLOGY
    .union(DirtyFlags::REPS)
    .union(DirtyFlags::VISIBILITY)
    .union(DirtyFlags::LOD);

pub(crate) const REPS: &[RepCatalogEntry] = &[
    RepCatalogEntry {
        kind: RepKind::Sphere,
        mask: RepMask::SPHERES,
        constructor: sphere,
        estimator: estimate_sphere,
        budget_invalidating_dirty: COUNT_BUDGET_DIRTY,
        cullable: true,
    },
    RepCatalogEntry {
        kind: RepKind::Stick,
        mask: RepMask::STICKS,
        constructor: stick,
        estimator: estimate_stick,
        budget_invalidating_dirty: COUNT_BUDGET_DIRTY,
        cullable: true,
    },
    RepCatalogEntry {
        kind: RepKind::Line,
        mask: RepMask::LINES,
        constructor: line,
        estimator: estimate_line,
        budget_invalidating_dirty: COUNT_BUDGET_DIRTY,
        cullable: true,
    },
    RepCatalogEntry {
        kind: RepKind::Dot,
        mask: RepMask::DOTS,
        constructor: dot,
        estimator: estimate_dot,
        budget_invalidating_dirty: COUNT_BUDGET_DIRTY,
        cullable: true,
    },
    RepCatalogEntry {
        kind: RepKind::Surface,
        mask: RepMask::SURFACE,
        constructor: surface,
        estimator: estimate_surface,
        budget_invalidating_dirty: COUNT_BUDGET_DIRTY,
        cullable: false,
    },
    RepCatalogEntry {
        kind: RepKind::Mesh,
        mask: RepMask::MESH,
        constructor: mesh,
        estimator: estimate_mesh,
        budget_invalidating_dirty: COUNT_BUDGET_DIRTY,
        cullable: false,
    },
    RepCatalogEntry {
        kind: RepKind::Cartoon,
        mask: RepMask::CARTOON,
        constructor: cartoon,
        estimator: estimate_cartoon,
        budget_invalidating_dirty: COUNT_BUDGET_DIRTY,
        cullable: false,
    },
    RepCatalogEntry {
        kind: RepKind::Ribbon,
        mask: RepMask::RIBBON,
        constructor: ribbon,
        estimator: estimate_ribbon,
        budget_invalidating_dirty: COUNT_BUDGET_DIRTY,
        cullable: false,
    },
    RepCatalogEntry {
        kind: RepKind::Ellipsoid,
        mask: RepMask::ELLIPSOIDS,
        constructor: ellipsoid,
        estimator: estimate_ellipsoid,
        budget_invalidating_dirty: COUNT_BUDGET_DIRTY,
        cullable: true,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Geometry reps (Surface/Mesh/Cartoon/Ribbon) now budget against counts
    /// only. COORDS-only changes must not invalidate their budget estimate:
    /// their estimators are cheap upper bounds that do not depend on exact
    /// coordinates, so the budget request cache can hit during coord reloads.
    #[test]
    fn geometry_reps_budget_against_counts_not_coords() {
        for kind in [
            RepKind::Surface,
            RepKind::Mesh,
            RepKind::Cartoon,
            RepKind::Ribbon,
        ] {
            let entry = entry(kind).expect("catalog entry");
            assert!(
                !entry.budget_estimate_invalidated_by(DirtyFlags::COORDS),
                "{kind:?} should not re-estimate on COORDS"
            );
            for dirty in [
                DirtyFlags::TOPOLOGY,
                DirtyFlags::REPS,
                DirtyFlags::VISIBILITY,
                DirtyFlags::LOD,
            ] {
                assert!(
                    entry.budget_estimate_invalidated_by(dirty),
                    "{kind:?} should re-estimate on {dirty:?}"
                );
            }
        }
    }

    /// No built-in rep budgets against COORDS any longer; `GEOMETRY_BUDGET_DIRTY`
    /// was removed. Verify the whole catalog agrees.
    #[test]
    fn no_rep_budget_estimate_invalidated_by_coords() {
        for entry in REPS {
            assert!(
                !entry.budget_estimate_invalidated_by(DirtyFlags::COORDS),
                "{:?} must not re-estimate on COORDS",
                entry.kind
            );
        }
    }
}
