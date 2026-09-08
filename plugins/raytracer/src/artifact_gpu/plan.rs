use patinae_scene::{
    RenderArtifactBufferDescriptor, RenderArtifactBufferRole, RenderArtifactPrimitiveTopology,
    RenderArtifactRepDescriptor, RenderArtifactRepKind, RenderArtifactSnapshotDescriptor,
};

use super::layout::{
    ATOM_STRIDE, COLOR_LUT_STRIDE, LINE_INSTANCE_STRIDE, MAX_ENCODED_PRIMITIVE_INDEX,
    RAY_LINE_RADIUS, SPHERE_INSTANCE_STRIDE, STD_VERTEX_STRIDE, STICK_INSTANCE_STRIDE,
};
use super::reps::{
    ArtifactPlan, CapsuleArtifactRep, CylinderArtifactRep, SphereArtifactRep, TriangleArtifactRep,
};

pub(super) fn plan_artifact_primitives<'a>(
    snapshot: &'a RenderArtifactSnapshotDescriptor,
) -> Result<ArtifactPlan<'a>, String> {
    if snapshot.layout_version != patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION {
        return Err(format!(
            "unsupported render artifact layout version {}; expected {}",
            snapshot.layout_version,
            patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        ));
    }
    let color_lut = snapshot
        .buffers
        .iter()
        .find(|buffer| buffer.role == RenderArtifactBufferRole::SceneColorLut)
        .ok_or_else(|| "render artifact snapshot is missing SceneColorLut".to_string())?;
    validate_color_lut(color_lut)?;
    let scene_atoms = snapshot
        .buffers
        .iter()
        .find(|buffer| buffer.role == RenderArtifactBufferRole::SceneAtoms);
    if let Some(scene_atoms) = scene_atoms {
        validate_scene_atoms(scene_atoms)?;
    }

    let mut sphere_reps = Vec::new();
    let mut cylinder_reps = Vec::new();
    let mut capsule_reps = Vec::new();
    let mut triangle_reps = Vec::new();
    let mut sphere_count = 0_u32;
    let mut cylinder_count = 0_u32;
    let mut capsule_count = 0_u32;
    let mut triangle_count = 0_u32;
    for rep in &snapshot.reps {
        match rep.topology {
            RenderArtifactPrimitiveTopology::SphereInstances => {
                let Some(rep_slot) = sphere_rep_slot(rep.rep_kind) else {
                    reject_nonempty_rep(rep)?;
                    continue;
                };
                let instance_capacity = instance_capacity_for(rep, snapshot, "sphere")?;
                if instance_capacity == 0 {
                    continue;
                }
                let instance_capacity = effective_rep_geometry_elements(
                    snapshot,
                    rep,
                    RenderArtifactBufferRole::SphereInstances,
                    SPHERE_INSTANCE_STRIDE,
                    instance_capacity,
                    "sphere",
                )?;
                if instance_capacity == 0 {
                    continue;
                }
                let geometry_binding_size =
                    binding_size_for(instance_capacity, SPHERE_INSTANCE_STRIDE, "sphere geometry")?;
                ensure_encoded_index(instance_capacity, "sphere")?;
                sphere_reps.push(SphereArtifactRep {
                    rep,
                    sphere_offset: sphere_count,
                    instance_capacity,
                    geometry_binding_size,
                    rep_slot,
                });
                sphere_count =
                    checked_add(sphere_count, instance_capacity, "artifact sphere count")?;
            }
            RenderArtifactPrimitiveTopology::CylinderInstances => {
                let Some(rep_slot) = capsule_rep_slot(rep.rep_kind) else {
                    reject_nonempty_rep(rep)?;
                    continue;
                };
                let instance_capacity = instance_capacity_for(rep, snapshot, "capsule")?;
                if instance_capacity == 0 {
                    continue;
                }
                let instance_capacity = effective_rep_geometry_elements(
                    snapshot,
                    rep,
                    RenderArtifactBufferRole::StickInstances,
                    STICK_INSTANCE_STRIDE,
                    instance_capacity,
                    "capsule",
                )?;
                if instance_capacity == 0 {
                    continue;
                }
                let geometry_binding_size =
                    binding_size_for(instance_capacity, STICK_INSTANCE_STRIDE, "capsule geometry")?;
                ensure_encoded_index(instance_capacity, "capsule")?;
                capsule_reps.push(CapsuleArtifactRep {
                    rep,
                    capsule_offset: capsule_count,
                    instance_capacity,
                    geometry_binding_size,
                    rep_slot,
                });
                capsule_count =
                    checked_add(capsule_count, instance_capacity, "artifact capsule count")?;
            }
            RenderArtifactPrimitiveTopology::LineInstances => {
                let Some(rep_slot) = line_rep_slot(rep.rep_kind) else {
                    reject_nonempty_rep(rep)?;
                    continue;
                };
                let instance_capacity = instance_capacity_for(rep, snapshot, "line")?;
                if instance_capacity == 0 {
                    continue;
                }
                let instance_capacity = effective_rep_geometry_elements(
                    snapshot,
                    rep,
                    RenderArtifactBufferRole::LineInstances,
                    LINE_INSTANCE_STRIDE,
                    instance_capacity,
                    "line",
                )?;
                if instance_capacity == 0 {
                    continue;
                }
                let geometry_binding_size =
                    binding_size_for(instance_capacity, LINE_INSTANCE_STRIDE, "line geometry")?;
                ensure_encoded_index(instance_capacity, "line")?;
                cylinder_reps.push(CylinderArtifactRep {
                    rep,
                    cylinder_offset: cylinder_count,
                    instance_capacity,
                    geometry_binding_size,
                    rep_slot,
                    radius: RAY_LINE_RADIUS,
                });
                cylinder_count =
                    checked_add(cylinder_count, instance_capacity, "artifact cylinder count")?;
            }
            RenderArtifactPrimitiveTopology::TriangleList => {
                let Some(rep_slot) = direct_triangle_rep_slot(rep.rep_kind) else {
                    reject_nonempty_rep(rep)?;
                    continue;
                };
                if rep.count.is_some() {
                    return Err(format!(
                        "native GPU artifact ray path does not support count-buffer {:?} triangle artifacts",
                        rep.rep_kind
                    ));
                }
                let requested_vertex_count = triangle_vertex_count_for(rep, snapshot)?;
                if requested_vertex_count == 0 {
                    continue;
                }
                if rep.indirect.is_none() && requested_vertex_count % 3 != 0 {
                    return Err(format!(
                        "{:?} artifact vertex count {} is not divisible by 3",
                        rep.rep_kind, requested_vertex_count
                    ));
                }
                let effective_vertex_count = effective_rep_geometry_elements(
                    snapshot,
                    rep,
                    RenderArtifactBufferRole::StdVertices,
                    STD_VERTEX_STRIDE,
                    requested_vertex_count,
                    "triangle",
                )?;
                if rep.indirect.is_none() && effective_vertex_count % 3 != 0 {
                    return Err(format!(
                        "{:?} artifact vertex count {} is not divisible by 3",
                        rep.rep_kind, effective_vertex_count
                    ));
                }
                let vertex_count = if rep.indirect.is_some() {
                    effective_vertex_count / 3 * 3
                } else {
                    effective_vertex_count
                };
                if vertex_count == 0 {
                    continue;
                }
                let rep_triangles = vertex_count / 3;
                let geometry_binding_size =
                    binding_size_for(vertex_count, STD_VERTEX_STRIDE, "triangle geometry")?;
                ensure_encoded_index(rep_triangles, "triangle")?;
                triangle_reps.push(TriangleArtifactRep {
                    rep,
                    triangle_offset: triangle_count,
                    triangle_count: rep_triangles,
                    vertex_count,
                    geometry_binding_size,
                    rep_slot,
                    visibility_counter_index: None,
                });
                triangle_count = triangle_count
                    .checked_add(rep_triangles)
                    .ok_or_else(|| "artifact triangle count overflow".to_string())?;
            }
            RenderArtifactPrimitiveTopology::LineList => {
                if rep.max_element_count > 0 || rep.element_count > 0 {
                    return Err(format!(
                        "native GPU artifact ray path does not yet support {:?} line-list artifacts",
                        rep.rep_kind
                    ));
                }
            }
        }
    }

    if !triangle_reps.is_empty() && scene_atoms.is_none() {
        return Err(
            "render artifact snapshot is missing SceneAtoms for triangle artifacts".to_string(),
        );
    }

    Ok(ArtifactPlan {
        color_lut: color_lut.handle,
        scene_atoms: scene_atoms.map(|buffer| buffer.handle),
        sphere_reps,
        cylinder_reps,
        capsule_reps,
        triangle_reps,
        sphere_count,
        cylinder_count,
        capsule_count,
        triangle_count,
    })
}

fn validate_color_lut(buffer: &RenderArtifactBufferDescriptor) -> Result<(), String> {
    if buffer.stride != COLOR_LUT_STRIDE {
        return Err(format!(
            "SceneColorLut stride {} does not match expected {}",
            buffer.stride, COLOR_LUT_STRIDE
        ));
    }
    if buffer.size < COLOR_LUT_STRIDE || buffer.element_count == 0 {
        return Err(format!(
            "SceneColorLut buffer size {} is too small for stride {}",
            buffer.size, COLOR_LUT_STRIDE
        ));
    }
    Ok(())
}

fn validate_scene_atoms(buffer: &RenderArtifactBufferDescriptor) -> Result<(), String> {
    if buffer.stride != ATOM_STRIDE {
        return Err(format!(
            "SceneAtoms stride {} does not match expected {}",
            buffer.stride, ATOM_STRIDE
        ));
    }
    if buffer.size < ATOM_STRIDE || buffer.element_count == 0 {
        return Err(format!(
            "SceneAtoms buffer size {} is too small for stride {}",
            buffer.size, ATOM_STRIDE
        ));
    }
    Ok(())
}

fn effective_rep_geometry_elements(
    snapshot: &RenderArtifactSnapshotDescriptor,
    rep: &RenderArtifactRepDescriptor,
    expected_role: RenderArtifactBufferRole,
    expected_stride: u64,
    requested_elements: u32,
    label: &str,
) -> Result<u32, String> {
    let buffer = snapshot
        .buffers
        .iter()
        .find(|buffer| buffer.handle == rep.geometry)
        .ok_or_else(|| {
            format!(
                "{:?} {label} artifact geometry buffer {:?} is missing from the snapshot",
                rep.rep_kind, rep.geometry
            )
        })?;
    if buffer.role != expected_role {
        return Err(format!(
            "{:?} {label} artifact geometry buffer role {:?} does not match expected {:?}",
            rep.rep_kind, buffer.role, expected_role
        ));
    }
    if buffer.stride != expected_stride {
        return Err(format!(
            "{:?} {label} artifact geometry stride {} does not match expected {}",
            rep.rep_kind, buffer.stride, expected_stride
        ));
    }
    let physical_elements = buffer.size / expected_stride;
    let available_elements = buffer.element_count.min(physical_elements);
    let effective_elements = u64::from(requested_elements).min(available_elements);
    if effective_elements < u64::from(requested_elements) {
        log::warn!(
            "native GPU artifact ray path clamped {:?} {} geometry from {} to {} elements because buffer size={} stride={} element_count={}",
            rep.rep_kind,
            label,
            requested_elements,
            effective_elements,
            buffer.size,
            expected_stride,
            buffer.element_count
        );
    }
    u32::try_from(effective_elements).map_err(|_| {
        format!(
            "{:?} {label} artifact geometry capacity exceeds u32",
            rep.rep_kind
        )
    })
}

fn sphere_rep_slot(kind: RenderArtifactRepKind) -> Option<u32> {
    match kind {
        RenderArtifactRepKind::Sphere => Some(0),
        _ => None,
    }
}

fn capsule_rep_slot(kind: RenderArtifactRepKind) -> Option<u32> {
    match kind {
        RenderArtifactRepKind::Stick => Some(1),
        _ => None,
    }
}

fn line_rep_slot(kind: RenderArtifactRepKind) -> Option<u32> {
    match kind {
        RenderArtifactRepKind::Line => Some(2),
        _ => None,
    }
}

fn direct_triangle_rep_slot(kind: RenderArtifactRepKind) -> Option<u32> {
    match kind {
        RenderArtifactRepKind::Cartoon => Some(4),
        RenderArtifactRepKind::Ribbon => Some(5),
        RenderArtifactRepKind::Surface => Some(6),
        _ => None,
    }
}

fn reject_nonempty_rep(rep: &RenderArtifactRepDescriptor) -> Result<(), String> {
    if rep.max_element_count > 0 || rep.element_count > 0 {
        return Err(format!(
            "native GPU artifact ray path does not yet support {:?} {:?} artifacts",
            rep.rep_kind, rep.topology
        ));
    }
    Ok(())
}

fn instance_capacity_for(
    rep: &RenderArtifactRepDescriptor,
    snapshot: &RenderArtifactSnapshotDescriptor,
    label: &str,
) -> Result<u32, String> {
    if rep.indirect.is_some() {
        if !snapshot.cull_pass_initialized {
            return Err(format!(
                "render artifact snapshot cull pass is not initialized for {:?} {} artifacts",
                rep.rep_kind, label
            ));
        }
        return u32::try_from(rep.max_element_count)
            .map_err(|_| format!("{label} artifact capacity exceeds u32"));
    }
    if rep.count.is_some() {
        return Err(format!(
            "native GPU artifact ray path requires indirect draw args for {:?} {} artifacts",
            rep.rep_kind, label
        ));
    }
    u32::try_from(rep.element_count).map_err(|_| format!("{label} artifact count exceeds u32"))
}

fn triangle_vertex_count_for(
    rep: &RenderArtifactRepDescriptor,
    _snapshot: &RenderArtifactSnapshotDescriptor,
) -> Result<u32, String> {
    let count = if rep.indirect.is_some() {
        rep.max_element_count
    } else {
        rep.element_count
    };
    u32::try_from(count).map_err(|_| "artifact vertex count exceeds u32".to_string())
}

fn ensure_encoded_index(count: u32, label: &str) -> Result<(), String> {
    if count > MAX_ENCODED_PRIMITIVE_INDEX {
        return Err(format!(
            "{label} primitive count {count} exceeds BVH encoding limit {MAX_ENCODED_PRIMITIVE_INDEX}",
        ));
    }
    Ok(())
}

fn checked_add(lhs: u32, rhs: u32, label: &str) -> Result<u32, String> {
    lhs.checked_add(rhs)
        .ok_or_else(|| format!("{label} overflow"))
}

fn binding_size_for(elements: u32, stride: u64, label: &str) -> Result<u64, String> {
    u64::from(elements)
        .checked_mul(stride)
        .ok_or_else(|| format!("{label} binding size overflow"))
}
