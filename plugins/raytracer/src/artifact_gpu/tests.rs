use super::layout::{
    ArtifactPrimitiveMetadata, ArtifactTriangle, ArtifactVisibleTriangleParams, ATOM_STRIDE,
    COLOR_LUT_STRIDE, EMPTY_STORAGE_BYTES, LINE_INSTANCE_STRIDE, RAY_LINE_RADIUS,
    SPHERE_INSTANCE_STRIDE, STD_VERTEX_STRIDE, STICK_INSTANCE_STRIDE, WORKGROUP_SIZE,
};
use super::resources::{checked_storage_buffer_size, storage_bytes_for_device};
use super::*;
use crate::primitive::GpuTriangle;
use crate::shaders;
use patinae_render::mib_to_bytes;
use patinae_scene::{
    GpuBufferUsage, GpuHandle, GpuHandleKind, RenderArtifactBufferDescriptor,
    RenderArtifactBufferRole, RenderArtifactPrimitiveTopology, RenderArtifactRepDescriptor,
    RenderArtifactRepKind, RenderArtifactSnapshotDescriptor,
};

fn handle(id: u64) -> GpuHandle {
    GpuHandle {
        id,
        kind: GpuHandleKind::Buffer,
        generation: 1,
    }
}

fn fixture_limits(
    max_buffer_size: u64,
    max_storage_buffer_binding_size: u64,
) -> patinae_scene::GpuDeviceLimits {
    patinae_scene::GpuDeviceLimits {
        max_buffer_size,
        max_storage_buffer_binding_size,
        max_compute_workgroups_per_dimension: 65_535,
        max_compute_invocations_per_workgroup: 256,
        max_compute_workgroup_size_x: 256,
        max_compute_workgroup_size_y: 1,
        max_compute_workgroup_size_z: 1,
        buffer_binding_array: false,
        storage_resource_binding_array: false,
    }
}

fn scene_atoms_descriptor(id: u64, element_count: u64) -> RenderArtifactBufferDescriptor {
    RenderArtifactBufferDescriptor {
        handle: handle(id),
        role: RenderArtifactBufferRole::SceneAtoms,
        size: element_count * ATOM_STRIDE,
        stride: ATOM_STRIDE,
        element_count,
    }
}

// Dispatch, layout, and storage sizing.

#[test]
fn bvh_shape_pads_leaves_to_power_of_two() {
    let shape = bvh::bvh_shape_for(9).expect("shape");

    assert_eq!(shape.leaf_slots, 4);
    assert_eq!(shape.leaf_start, 3);
    assert_eq!(shape.node_count, 7);
}

#[test]
fn dispatch_grid_tiles_past_single_dimension_limit() {
    let items = 90_473 * WORKGROUP_SIZE;
    let grid = dispatch::dispatch_grid_for(items, 65_535).expect("dispatch grid");

    assert_eq!(grid.workgroups, [65_535, 2, 1]);
    assert_eq!(grid.invocation_width, 65_535 * WORKGROUP_SIZE);
}

#[test]
fn typed_storage_buffers_keep_one_element_for_empty_arrays() {
    assert_eq!(
        resources::storage_bytes_for::<GpuSphere>(0, "spheres").expect("sphere bytes"),
        std::mem::size_of::<GpuSphere>() as u64
    );
    assert_eq!(
        resources::storage_bytes_for::<GpuCylinder>(0, "cylinders").expect("cylinder bytes"),
        std::mem::size_of::<GpuCylinder>() as u64
    );
    assert_eq!(
        resources::storage_bytes_for::<GpuCapsule>(0, "capsules").expect("capsule bytes"),
        std::mem::size_of::<GpuCapsule>() as u64
    );
    assert_eq!(
        resources::storage_bytes_for::<GpuTriangle>(0, "triangles").expect("triangle bytes"),
        std::mem::size_of::<GpuTriangle>() as u64
    );
    assert_eq!(
        resources::storage_bytes_for::<ArtifactTriangle>(0, "artifact triangles")
            .expect("artifact triangle bytes"),
        std::mem::size_of::<ArtifactTriangle>() as u64
    );
}

#[test]
fn artifact_triangle_layout_is_compact() {
    assert_eq!(std::mem::size_of::<ArtifactTriangle>(), 80);
}

#[test]
fn finalize_streaming_metadata_params_layout_matches_wgsl() {
    assert_eq!(
        std::mem::size_of::<super::layout::FinalizeStreamingMetadataParams>(),
        32
    );
}

#[test]
fn storage_indirect_usage_includes_indirect_for_gpu_written_dispatch_args() {
    let usage = resources::storage_indirect_usage();

    assert!(usage.contains(GpuBufferUsage::STORAGE));
    assert!(usage.contains(GpuBufferUsage::INDIRECT));
    assert!(usage.contains(GpuBufferUsage::COPY_DST));
}

#[test]
fn artifact_triangle_storage_fits_reported_3j3q_visible_count() {
    let limits = fixture_limits(4_294_967_296, 2_147_483_647);
    let visible_triangles = 25_174_294;

    let compact_bytes = storage_bytes_for_device::<ArtifactTriangle>(
        visible_triangles,
        &limits,
        "ray artifact triangles",
    )
    .expect("compact artifact triangle storage fits");

    assert_eq!(compact_bytes, 2_013_943_520);
    assert!(storage_bytes_for_device::<GpuTriangle>(
        visible_triangles,
        &limits,
        "ray artifact triangles",
    )
    .unwrap_err()
    .contains(
        "ray artifact triangles buffer size 3222309632 exceeds GPU storage buffer limit 2147483647"
    ));
}

#[test]
fn storage_buffer_size_check_uses_storage_binding_limit() {
    let limits = fixture_limits(1024, 512);

    assert!(checked_storage_buffer_size(512, &limits, "test storage").is_ok());
    let err = checked_storage_buffer_size(513, &limits, "test storage").unwrap_err();

    assert!(err.contains("test storage buffer size 513 exceeds GPU storage buffer limit 512"));
}

#[test]
fn planner_selects_streaming_fallback_for_reported_3j3q_surface_capacity() {
    let source_triangles = 143_029_960_u64;
    let source_vertices = source_triangles * 3;
    let limits = fixture_limits(4_294_967_296, 2_147_483_647);
    let snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 7,
        scene_bounds_min: [0.0, 0.0, 0.0],
        scene_bounds_max: [1.0, 1.0, 1.0],
        cull_pass_initialized: true,
        device_limits: limits,
        buffers: vec![
            RenderArtifactBufferDescriptor {
                handle: handle(10),
                role: RenderArtifactBufferRole::SceneColorLut,
                size: 64,
                stride: COLOR_LUT_STRIDE,
                element_count: 1,
            },
            scene_atoms_descriptor(90, 16),
            RenderArtifactBufferDescriptor {
                handle: handle(11),
                role: RenderArtifactBufferRole::StdVertices,
                size: source_vertices * STD_VERTEX_STRIDE,
                stride: STD_VERTEX_STRIDE,
                element_count: source_vertices,
            },
        ],
        reps: vec![RenderArtifactRepDescriptor {
            object_id: 1,
            rep_kind: RenderArtifactRepKind::Surface,
            topology: RenderArtifactPrimitiveTopology::TriangleList,
            geometry: handle(11),
            count: None,
            indirect: Some(handle(12)),
            element_count: 0,
            max_element_count: source_vertices,
            atom_offset: 5,
            atom_count: 8,
            material_rgba: [0.5, 0.5, 0.5, 1.0],
            transparency: 0.5,
        }],
    };
    let mut plan = plan::plan_artifact_primitives(&snapshot).expect("plan");
    triangles::prepare_triangle_gpu_metadata(&mut plan).expect("surface metadata");

    assert_eq!(plan.triangle_count, source_triangles as u32);
    assert_eq!(
        streaming::choose_artifact_ray_plan(&plan, &limits),
        streaming::ArtifactRayPlan::StreamingFallback
    );
    let shape = streaming::choose_streaming_chunk_shape(
        &plan,
        &limits,
        limits.max_compute_workgroups_per_dimension,
    )
    .expect("streaming chunk shape");

    assert!(shape.triangle_capacity < plan.triangle_count);
    assert!(
        u64::from(shape.triangle_capacity) * std::mem::size_of::<ArtifactTriangle>() as u64
            <= limits.max_storage_buffer_binding_size
    );
}

#[test]
fn streaming_chunk_uses_storage_limit_not_largest_source_rep() {
    let direct_triangles = 64_u64;
    let surface_triangles = 64_u64;
    let limits = fixture_limits(4_294_967_296, 2_147_483_647);
    let snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 7,
        scene_bounds_min: [0.0, 0.0, 0.0],
        scene_bounds_max: [1.0, 1.0, 1.0],
        cull_pass_initialized: true,
        device_limits: limits,
        buffers: vec![
            RenderArtifactBufferDescriptor {
                handle: handle(10),
                role: RenderArtifactBufferRole::SceneColorLut,
                size: 64,
                stride: COLOR_LUT_STRIDE,
                element_count: 1,
            },
            scene_atoms_descriptor(90, 16),
            RenderArtifactBufferDescriptor {
                handle: handle(11),
                role: RenderArtifactBufferRole::StdVertices,
                size: direct_triangles * 3 * STD_VERTEX_STRIDE,
                stride: STD_VERTEX_STRIDE,
                element_count: direct_triangles * 3,
            },
            RenderArtifactBufferDescriptor {
                handle: handle(12),
                role: RenderArtifactBufferRole::StdVertices,
                size: surface_triangles * 3 * STD_VERTEX_STRIDE,
                stride: STD_VERTEX_STRIDE,
                element_count: surface_triangles * 3,
            },
        ],
        reps: vec![
            RenderArtifactRepDescriptor {
                object_id: 1,
                rep_kind: RenderArtifactRepKind::Cartoon,
                topology: RenderArtifactPrimitiveTopology::TriangleList,
                geometry: handle(11),
                count: None,
                indirect: None,
                element_count: direct_triangles * 3,
                max_element_count: direct_triangles * 3,
                atom_offset: 5,
                atom_count: 8,
                material_rgba: [0.5, 0.5, 0.5, 1.0],
                transparency: 0.0,
            },
            RenderArtifactRepDescriptor {
                object_id: 1,
                rep_kind: RenderArtifactRepKind::Surface,
                topology: RenderArtifactPrimitiveTopology::TriangleList,
                geometry: handle(12),
                count: None,
                indirect: Some(handle(13)),
                element_count: 0,
                max_element_count: surface_triangles * 3,
                atom_offset: 5,
                atom_count: 8,
                material_rgba: [0.5, 0.5, 0.5, 1.0],
                transparency: 0.5,
            },
        ],
    };
    let mut plan = plan::plan_artifact_primitives(&snapshot).expect("plan");
    triangles::prepare_triangle_gpu_metadata(&mut plan).expect("surface metadata");

    let shape = streaming::choose_streaming_chunk_shape(
        &plan,
        &limits,
        limits.max_compute_workgroups_per_dimension,
    )
    .expect("streaming chunk shape");
    let budget =
        streaming::streaming_triangle_budget(&plan, shape.triangle_capacity).expect("budget");

    assert!(shape.triangle_capacity > direct_triangles as u32);
    assert!(budget.include_direct_triangles);
    assert!(budget.visible_triangle_capacity > 0);
}

#[test]
fn streaming_budget_reserves_surface_capacity_when_direct_fills_chunk() {
    let direct_triangles = 64_u64;
    let surface_triangles = 64_u64;
    let limits = fixture_limits(4_294_967_296, 2_147_483_647);
    let snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 7,
        scene_bounds_min: [0.0, 0.0, 0.0],
        scene_bounds_max: [1.0, 1.0, 1.0],
        cull_pass_initialized: true,
        device_limits: limits,
        buffers: vec![
            RenderArtifactBufferDescriptor {
                handle: handle(10),
                role: RenderArtifactBufferRole::SceneColorLut,
                size: 64,
                stride: COLOR_LUT_STRIDE,
                element_count: 1,
            },
            scene_atoms_descriptor(90, 16),
            RenderArtifactBufferDescriptor {
                handle: handle(11),
                role: RenderArtifactBufferRole::StdVertices,
                size: direct_triangles * 3 * STD_VERTEX_STRIDE,
                stride: STD_VERTEX_STRIDE,
                element_count: direct_triangles * 3,
            },
            RenderArtifactBufferDescriptor {
                handle: handle(12),
                role: RenderArtifactBufferRole::StdVertices,
                size: surface_triangles * 3 * STD_VERTEX_STRIDE,
                stride: STD_VERTEX_STRIDE,
                element_count: surface_triangles * 3,
            },
        ],
        reps: vec![
            RenderArtifactRepDescriptor {
                object_id: 1,
                rep_kind: RenderArtifactRepKind::Cartoon,
                topology: RenderArtifactPrimitiveTopology::TriangleList,
                geometry: handle(11),
                count: None,
                indirect: None,
                element_count: direct_triangles * 3,
                max_element_count: direct_triangles * 3,
                atom_offset: 5,
                atom_count: 8,
                material_rgba: [0.5, 0.5, 0.5, 1.0],
                transparency: 0.0,
            },
            RenderArtifactRepDescriptor {
                object_id: 1,
                rep_kind: RenderArtifactRepKind::Surface,
                topology: RenderArtifactPrimitiveTopology::TriangleList,
                geometry: handle(12),
                count: None,
                indirect: Some(handle(13)),
                element_count: 0,
                max_element_count: surface_triangles * 3,
                atom_offset: 5,
                atom_count: 8,
                material_rgba: [0.5, 0.5, 0.5, 1.0],
                transparency: 0.5,
            },
        ],
    };
    let mut plan = plan::plan_artifact_primitives(&snapshot).expect("plan");
    triangles::prepare_triangle_gpu_metadata(&mut plan).expect("surface metadata");

    let budget = streaming::streaming_triangle_budget(&plan, direct_triangles as u32)
        .expect("surface-first budget");

    assert!(!budget.include_direct_triangles);
    assert_eq!(budget.direct_triangle_count, 0);
    assert_eq!(
        budget.skipped_direct_triangle_count,
        direct_triangles as u32
    );
    assert_eq!(budget.visible_triangle_capacity, direct_triangles as u32);
}

// Visibility and indirect draw helpers.

#[test]
fn visible_triangle_params_uniform_layout_matches_wgsl() {
    assert_eq!(std::mem::size_of::<ArtifactVisibleTriangleParams>(), 176);
}

#[test]
fn primitive_metadata_storage_layout_matches_wgsl() {
    assert_eq!(std::mem::size_of::<ArtifactPrimitiveMetadata>(), 32);
}

// WGSL expansion and validation.

#[test]
fn primitive_metadata_readback_decodes_layout() {
    let metadata = ArtifactPrimitiveMetadata {
        sphere_count: 1,
        cylinder_count: 2,
        capsule_count: 3,
        triangle_count: 4,
        primitive_count: 10,
        triangle_capacity: 12,
        visible_triangle_count: 4,
        overflow: 0,
    };

    let decoded = dispatch::decode_primitive_metadata_readback(bytemuck::bytes_of(&metadata))
        .expect("metadata readback");

    assert_eq!(decoded.sphere_count, 1);
    assert_eq!(decoded.primitive_count, 10);
    assert_eq!(decoded.visible_triangle_count, 4);
}

#[test]
fn raytrace_readback_split_preserves_export_pixels_after_debug_metadata() {
    let metadata = ArtifactPrimitiveMetadata {
        sphere_count: 1,
        cylinder_count: 0,
        capsule_count: 0,
        triangle_count: 7,
        primitive_count: 8,
        triangle_capacity: 16,
        visible_triangle_count: 7,
        overflow: 1,
    };
    let pixels = vec![9_u8; 16];

    let readbacks = dispatch::split_raytrace_readbacks(
        dispatch::RaytraceDispatchTarget::CpuReadback,
        true,
        vec![bytemuck::bytes_of(&metadata).to_vec(), pixels.clone()],
    )
    .expect("split readbacks");

    let decoded = readbacks.metadata.expect("metadata");
    assert_eq!(decoded.triangle_count, 7);
    assert_eq!(decoded.overflow, 1);
    assert_eq!(readbacks.pixels.expect("pixels"), pixels);
}

#[test]
fn raytrace_viewport_readback_split_allows_zero_default_readbacks() {
    let readbacks = dispatch::split_raytrace_readbacks(
        dispatch::RaytraceDispatchTarget::ViewportGpu,
        false,
        Vec::new(),
    )
    .expect("split readbacks");

    assert!(readbacks.metadata.is_none());
    assert!(readbacks.pixels.is_none());
}

#[test]
fn artifact_wgsl_modules_parse_and_validate() {
    let modules: [(&str, String); 11] = [
        ("ray.main", shaders::RAYTRACE.to_string()),
        (
            "ray.standalone.downsample",
            shaders::STANDALONE_DOWNSAMPLE.to_string(),
        ),
        ("ray.artifact.spheres", shaders::artifact_spheres()),
        (
            "ray.artifact.stick_capsules",
            shaders::artifact_stick_capsules(),
        ),
        (
            "ray.artifact.line_cylinders",
            shaders::artifact_line_cylinders(),
        ),
        ("ray.artifact.triangles", shaders::artifact_triangles()),
        (
            "ray.artifact.visible_triangles",
            shaders::artifact_visible_triangles(),
        ),
        (
            "ray.artifact.finalize_streaming_metadata",
            shaders::artifact_finalize_streaming_metadata(),
        ),
        ("ray.artifact.bvh", shaders::artifact_bvh()),
        (
            "ray.artifact.raytrace",
            shaders::artifact_raytrace_output_buffer(),
        ),
        (
            "ray.artifact.downsample",
            shaders::ARTIFACT_DOWNSAMPLE.to_string(),
        ),
    ];

    let mut failures = Vec::new();

    for (label, source) in modules {
        let module = match naga::front::wgsl::parse_str(&source) {
            Ok(module) => module,
            Err(err) => {
                failures.push(format!(
                    "{label}: WGSL parse failed\n{}",
                    err.emit_to_string_with_path(&source, label)
                ));
                continue;
            }
        };

        for (_, ty) in module.types.iter() {
            if ty.name.as_deref() == Some("AtomGpu") {
                let naga::TypeInner::Struct { ref members, span } = ty.inner else {
                    panic!("{label}: AtomGpu must be a struct");
                };
                assert_eq!(u64::from(span), ATOM_STRIDE, "{label}: atom stride");
                assert_eq!(
                    span as usize,
                    std::mem::size_of::<patinae_render::scene_store::AtomGpu>()
                );
                assert_eq!(
                    members
                        .iter()
                        .map(|m| (m.name.as_deref(), m.offset))
                        .collect::<Vec<_>>(),
                    [
                        (Some("vdw"), 0),
                        (Some("repr_flags"), 4),
                        (Some("alpha_pack_a"), 8),
                        (Some("alpha_pack_b"), 12)
                    ]
                );
            }
        }
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        if let Err(err) = validator.validate(&module) {
            failures.push(format!("{label}: WGSL validation failed\n{err:#?}"));
        }
    }

    assert!(
        failures.is_empty(),
        "invalid ray artifact WGSL modules:\n{}",
        failures.join("\n\n")
    );
}

// Artifact planning.

#[test]
fn artifact_plan_rejects_incompatible_atom_layouts() {
    let mut snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 0,
        scene_bounds_min: [0.0; 3],
        scene_bounds_max: [1.0; 3],
        cull_pass_initialized: true,
        device_limits: fixture_limits(1024, 1024),
        buffers: vec![
            RenderArtifactBufferDescriptor {
                handle: handle(10),
                role: RenderArtifactBufferRole::SceneColorLut,
                size: 64,
                stride: COLOR_LUT_STRIDE,
                element_count: 1,
            },
            scene_atoms_descriptor(11, 2),
        ],
        reps: Vec::new(),
    };
    assert!(plan::plan_artifact_primitives(&snapshot).is_ok());
    snapshot.buffers[1].stride = 32;
    let error = plan::plan_artifact_primitives(&snapshot).err().unwrap();
    assert!(error.contains("SceneAtoms stride 32"));
    snapshot.buffers[1].stride = ATOM_STRIDE;
    for version in [2, u32::MAX] {
        snapshot.layout_version = version;
        let error = plan::plan_artifact_primitives(&snapshot).err().unwrap();
        assert!(error.contains("unsupported render artifact layout version"));
    }
}

#[test]
fn artifact_plan_uses_scene_color_lut_and_cartoon_slot() {
    let snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 7,
        scene_bounds_min: [0.0, 0.0, 0.0],
        scene_bounds_max: [1.0, 1.0, 1.0],
        cull_pass_initialized: true,
        device_limits: fixture_limits(1024, 1024),
        buffers: vec![
            RenderArtifactBufferDescriptor {
                handle: handle(10),
                role: RenderArtifactBufferRole::SceneColorLut,
                size: 64,
                stride: COLOR_LUT_STRIDE,
                element_count: 1,
            },
            scene_atoms_descriptor(90, 8),
            RenderArtifactBufferDescriptor {
                handle: handle(11),
                role: RenderArtifactBufferRole::StdVertices,
                size: 72,
                stride: STD_VERTEX_STRIDE,
                element_count: 3,
            },
        ],
        reps: vec![RenderArtifactRepDescriptor {
            object_id: 1,
            rep_kind: RenderArtifactRepKind::Cartoon,
            topology: RenderArtifactPrimitiveTopology::TriangleList,
            geometry: handle(11),
            count: None,
            indirect: None,
            element_count: 3,
            max_element_count: 3,
            atom_offset: 5,
            atom_count: 1,
            material_rgba: [1.0, 0.0, 0.0, 1.0],
            transparency: 0.0,
        }],
    };

    let plan = plan::plan_artifact_primitives(&snapshot).expect("plan");

    assert_eq!(plan.color_lut, handle(10));
    assert_eq!(plan.scene_atoms, Some(handle(90)));
    assert_eq!(plan.triangle_count, 1);
    assert_eq!(plan.triangle_reps[0].rep_slot, 4);
    assert_eq!(plan.primitive_count().expect("primitive count"), 1);
}

#[test]
fn artifact_plan_accepts_native_instance_artifacts() {
    let snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 7,
        scene_bounds_min: [0.0, 0.0, 0.0],
        scene_bounds_max: [1.0, 1.0, 1.0],
        cull_pass_initialized: true,
        device_limits: fixture_limits(4096, 4096),
        buffers: vec![
            RenderArtifactBufferDescriptor {
                handle: handle(10),
                role: RenderArtifactBufferRole::SceneColorLut,
                size: 64,
                stride: COLOR_LUT_STRIDE,
                element_count: 1,
            },
            RenderArtifactBufferDescriptor {
                handle: handle(11),
                role: RenderArtifactBufferRole::SphereInstances,
                size: 5 * SPHERE_INSTANCE_STRIDE,
                stride: SPHERE_INSTANCE_STRIDE,
                element_count: 5,
            },
            RenderArtifactBufferDescriptor {
                handle: handle(21),
                role: RenderArtifactBufferRole::StickInstances,
                size: 7 * STICK_INSTANCE_STRIDE,
                stride: STICK_INSTANCE_STRIDE,
                element_count: 7,
            },
            RenderArtifactBufferDescriptor {
                handle: handle(31),
                role: RenderArtifactBufferRole::LineInstances,
                size: 11 * LINE_INSTANCE_STRIDE,
                stride: LINE_INSTANCE_STRIDE,
                element_count: 11,
            },
        ],
        reps: vec![
            RenderArtifactRepDescriptor {
                object_id: 1,
                rep_kind: RenderArtifactRepKind::Sphere,
                topology: RenderArtifactPrimitiveTopology::SphereInstances,
                geometry: handle(11),
                count: Some(handle(12)),
                indirect: Some(handle(13)),
                element_count: 0,
                max_element_count: 5,
                atom_offset: 0,
                atom_count: 5,
                material_rgba: [1.0, 0.0, 0.0, 1.0],
                transparency: 0.0,
            },
            RenderArtifactRepDescriptor {
                object_id: 1,
                rep_kind: RenderArtifactRepKind::Stick,
                topology: RenderArtifactPrimitiveTopology::CylinderInstances,
                geometry: handle(21),
                count: Some(handle(22)),
                indirect: Some(handle(23)),
                element_count: 0,
                max_element_count: 7,
                atom_offset: 0,
                atom_count: 5,
                material_rgba: [0.0, 1.0, 0.0, 1.0],
                transparency: 0.25,
            },
            RenderArtifactRepDescriptor {
                object_id: 1,
                rep_kind: RenderArtifactRepKind::Line,
                topology: RenderArtifactPrimitiveTopology::LineInstances,
                geometry: handle(31),
                count: Some(handle(32)),
                indirect: Some(handle(33)),
                element_count: 0,
                max_element_count: 11,
                atom_offset: 0,
                atom_count: 5,
                material_rgba: [0.0, 0.0, 1.0, 1.0],
                transparency: 0.0,
            },
        ],
    };

    let plan = plan::plan_artifact_primitives(&snapshot).expect("plan");

    assert_eq!(plan.sphere_count, 5);
    assert_eq!(plan.cylinder_count, 11);
    assert_eq!(plan.capsule_count, 7);
    assert_eq!(plan.triangle_count, 0);
    assert_eq!(plan.sphere_reps[0].rep_slot, 0);
    assert_eq!(
        plan.sphere_reps[0].geometry_binding_size,
        5 * SPHERE_INSTANCE_STRIDE
    );
    assert_eq!(plan.capsule_reps[0].rep_slot, 1);
    assert_eq!(
        plan.capsule_reps[0].geometry_binding_size,
        7 * STICK_INSTANCE_STRIDE
    );
    assert_eq!(plan.cylinder_reps[0].rep_slot, 2);
    assert_eq!(plan.cylinder_reps[0].radius, RAY_LINE_RADIUS);
    assert_eq!(
        plan.cylinder_reps[0].geometry_binding_size,
        11 * LINE_INSTANCE_STRIDE
    );
    assert_eq!(plan.primitive_count().expect("primitive count"), 23);
}

#[test]
fn artifact_plan_skips_undersized_instance_geometry() {
    let snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 7,
        scene_bounds_min: [0.0, 0.0, 0.0],
        scene_bounds_max: [1.0, 1.0, 1.0],
        cull_pass_initialized: true,
        device_limits: fixture_limits(4096, 4096),
        buffers: vec![
            RenderArtifactBufferDescriptor {
                handle: handle(10),
                role: RenderArtifactBufferRole::SceneColorLut,
                size: 64,
                stride: COLOR_LUT_STRIDE,
                element_count: 1,
            },
            RenderArtifactBufferDescriptor {
                handle: handle(21),
                role: RenderArtifactBufferRole::StickInstances,
                size: EMPTY_STORAGE_BYTES,
                stride: STICK_INSTANCE_STRIDE,
                element_count: 1,
            },
        ],
        reps: vec![RenderArtifactRepDescriptor {
            object_id: 1,
            rep_kind: RenderArtifactRepKind::Stick,
            topology: RenderArtifactPrimitiveTopology::CylinderInstances,
            geometry: handle(21),
            count: Some(handle(22)),
            indirect: Some(handle(23)),
            element_count: 0,
            max_element_count: 1,
            atom_offset: 0,
            atom_count: 2,
            material_rgba: [0.0, 1.0, 0.0, 1.0],
            transparency: 0.0,
        }],
    };

    let plan = plan::plan_artifact_primitives(&snapshot).expect("plan");

    assert_eq!(plan.cylinder_count, 0);
    assert_eq!(plan.capsule_count, 0);
    assert!(plan.cylinder_reps.is_empty());
    assert!(plan.capsule_reps.is_empty());
    assert_eq!(plan.primitive_count().expect("primitive count"), 0);
}

#[test]
fn artifact_plan_accepts_indirect_surface_capacity() {
    let surface_capacity = 4_000_000;
    let snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 7,
        scene_bounds_min: [0.0, 0.0, 0.0],
        scene_bounds_max: [1.0, 1.0, 1.0],
        cull_pass_initialized: true,
        device_limits: fixture_limits(mib_to_bytes(128), mib_to_bytes(128)),
        buffers: vec![
            RenderArtifactBufferDescriptor {
                handle: handle(10),
                role: RenderArtifactBufferRole::SceneColorLut,
                size: 64,
                stride: COLOR_LUT_STRIDE,
                element_count: 1,
            },
            scene_atoms_descriptor(90, 16),
            RenderArtifactBufferDescriptor {
                handle: handle(11),
                role: RenderArtifactBufferRole::StdVertices,
                size: surface_capacity * STD_VERTEX_STRIDE,
                stride: STD_VERTEX_STRIDE,
                element_count: surface_capacity,
            },
        ],
        reps: vec![RenderArtifactRepDescriptor {
            object_id: 1,
            rep_kind: RenderArtifactRepKind::Surface,
            topology: RenderArtifactPrimitiveTopology::TriangleList,
            geometry: handle(11),
            count: None,
            indirect: Some(handle(12)),
            element_count: 0,
            max_element_count: surface_capacity,
            atom_offset: 5,
            atom_count: 8,
            material_rgba: [0.5, 0.5, 0.5, 1.0],
            transparency: 0.5,
        }],
    };

    let plan = plan::plan_artifact_primitives(&snapshot).expect("plan");

    assert_eq!(plan.triangle_count, 1_333_333);
    assert_eq!(plan.triangle_reps[0].rep_slot, 6);
    assert_eq!(plan.triangle_reps[0].vertex_count, 3_999_999);
    assert_eq!(plan.triangle_reps[0].visibility_counter_index, None);
    assert_eq!(
        plan.triangle_reps[0].geometry_binding_size,
        3_999_999 * STD_VERTEX_STRIDE
    );
}

// Surface visibility planning.

#[test]
fn surface_visibility_assigns_gpu_cursor_metadata_without_compacting_cpu_offsets() {
    let snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 7,
        scene_bounds_min: [0.0, 0.0, 0.0],
        scene_bounds_max: [1.0, 1.0, 1.0],
        cull_pass_initialized: true,
        device_limits: fixture_limits(4096, 4096),
        buffers: vec![
            RenderArtifactBufferDescriptor {
                handle: handle(10),
                role: RenderArtifactBufferRole::SceneColorLut,
                size: 64,
                stride: COLOR_LUT_STRIDE,
                element_count: 1,
            },
            scene_atoms_descriptor(90, 16),
            RenderArtifactBufferDescriptor {
                handle: handle(11),
                role: RenderArtifactBufferRole::StdVertices,
                size: 6 * STD_VERTEX_STRIDE,
                stride: STD_VERTEX_STRIDE,
                element_count: 6,
            },
            RenderArtifactBufferDescriptor {
                handle: handle(12),
                role: RenderArtifactBufferRole::StdVertices,
                size: 9 * STD_VERTEX_STRIDE,
                stride: STD_VERTEX_STRIDE,
                element_count: 9,
            },
        ],
        reps: vec![
            RenderArtifactRepDescriptor {
                object_id: 1,
                rep_kind: RenderArtifactRepKind::Cartoon,
                topology: RenderArtifactPrimitiveTopology::TriangleList,
                geometry: handle(11),
                count: None,
                indirect: None,
                element_count: 6,
                max_element_count: 6,
                atom_offset: 5,
                atom_count: 1,
                material_rgba: [1.0, 0.0, 0.0, 1.0],
                transparency: 0.0,
            },
            RenderArtifactRepDescriptor {
                object_id: 1,
                rep_kind: RenderArtifactRepKind::Surface,
                topology: RenderArtifactPrimitiveTopology::TriangleList,
                geometry: handle(12),
                count: None,
                indirect: None,
                element_count: 9,
                max_element_count: 9,
                atom_offset: 6,
                atom_count: 1,
                material_rgba: [0.5, 0.5, 0.5, 1.0],
                transparency: 0.25,
            },
        ],
    };
    let mut plan = plan::plan_artifact_primitives(&snapshot).expect("plan");

    let surface_rep_count =
        triangles::prepare_triangle_gpu_metadata(&mut plan).expect("surface metadata");

    assert_eq!(surface_rep_count, 1);
    assert_eq!(plan.triangle_count, 5);
    assert_eq!(plan.triangle_reps.len(), 2);
    assert_eq!(plan.triangle_reps[0].triangle_offset, 0);
    assert_eq!(plan.triangle_reps[0].triangle_count, 2);
    assert_eq!(plan.triangle_reps[0].visibility_counter_index, None);
    assert_eq!(plan.triangle_reps[1].triangle_offset, 2);
    assert_eq!(plan.triangle_reps[1].triangle_count, 3);
    assert_eq!(plan.triangle_reps[1].source_triangle_count(), 3);
    assert_eq!(plan.triangle_reps[1].visibility_counter_index, Some(0));
    assert_eq!(
        plan.triangle_reps[1].geometry_binding_size,
        9 * STD_VERTEX_STRIDE
    );
}

// Planning rejection paths.

#[test]
fn artifact_plan_rejects_direct_triangle_count_not_divisible_by_three() {
    let snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 7,
        scene_bounds_min: [0.0, 0.0, 0.0],
        scene_bounds_max: [1.0, 1.0, 1.0],
        cull_pass_initialized: true,
        device_limits: fixture_limits(4096, 4096),
        buffers: vec![
            RenderArtifactBufferDescriptor {
                handle: handle(10),
                role: RenderArtifactBufferRole::SceneColorLut,
                size: 64,
                stride: COLOR_LUT_STRIDE,
                element_count: 1,
            },
            RenderArtifactBufferDescriptor {
                handle: handle(11),
                role: RenderArtifactBufferRole::StdVertices,
                size: 4 * STD_VERTEX_STRIDE,
                stride: STD_VERTEX_STRIDE,
                element_count: 4,
            },
        ],
        reps: vec![RenderArtifactRepDescriptor {
            object_id: 1,
            rep_kind: RenderArtifactRepKind::Cartoon,
            topology: RenderArtifactPrimitiveTopology::TriangleList,
            geometry: handle(11),
            count: None,
            indirect: None,
            element_count: 4,
            max_element_count: 4,
            atom_offset: 5,
            atom_count: 1,
            material_rgba: [1.0, 0.0, 0.0, 1.0],
            transparency: 0.0,
        }],
    };

    let err = match plan::plan_artifact_primitives(&snapshot) {
        Ok(_) => panic!("plan should fail"),
        Err(err) => err,
    };

    assert!(err.contains("Cartoon artifact vertex count 4 is not divisible by 3"));
}

#[test]
fn artifact_plan_rejects_uninitialized_cull_counts() {
    let snapshot = RenderArtifactSnapshotDescriptor {
        snapshot_id: 1,
        layout_version: patinae_render::RENDER_ARTIFACT_LAYOUT_VERSION,
        scene_generation: 7,
        scene_bounds_min: [0.0, 0.0, 0.0],
        scene_bounds_max: [1.0, 1.0, 1.0],
        cull_pass_initialized: false,
        device_limits: fixture_limits(4096, 4096),
        buffers: vec![RenderArtifactBufferDescriptor {
            handle: handle(10),
            role: RenderArtifactBufferRole::SceneColorLut,
            size: 64,
            stride: COLOR_LUT_STRIDE,
            element_count: 1,
        }],
        reps: vec![RenderArtifactRepDescriptor {
            object_id: 1,
            rep_kind: RenderArtifactRepKind::Sphere,
            topology: RenderArtifactPrimitiveTopology::SphereInstances,
            geometry: handle(11),
            count: Some(handle(12)),
            indirect: Some(handle(13)),
            element_count: 0,
            max_element_count: 5,
            atom_offset: 0,
            atom_count: 5,
            material_rgba: [1.0, 0.0, 0.0, 1.0],
            transparency: 0.0,
        }],
    };

    let err = match plan::plan_artifact_primitives(&snapshot) {
        Ok(_) => panic!("plan should fail"),
        Err(err) => err,
    };

    assert!(err.contains("cull pass is not initialized"));
}

// Execute the expanded artifact shaders, using the same layouts as the host.
// Readbacks deliberately expose numeric results instead of shader source spelling.
struct ArtifactGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl ArtifactGpu {
    fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("explicit artifact GPU tests require an adapter; never silently skip");
        eprintln!("Artifact GPU adapter: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("artifact GPU device");
        Self { device, queue }
    }

    fn buffer<T: bytemuck::Pod>(&self, values: &[T], uniform: bool) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("artifact test fixture"),
                contents: bytemuck::cast_slice(values),
                usage: if uniform {
                    wgpu::BufferUsages::UNIFORM
                } else {
                    wgpu::BufferUsages::STORAGE
                } | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            })
    }

    fn dispatch(&self, source: &str, entry: &str, buffers: &[(u32, &wgpu::Buffer)]) {
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(entry),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            });
        let entries: Vec<_> = buffers
            .iter()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: *binding,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(entry),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        self.queue.submit([encoder.finish()]);
    }

    fn read<T: bytemuck::Pod>(&self, buffer: &wgpu::Buffer) -> Vec<T> {
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("artifact readback"),
            size: buffer.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
        self.queue.submit([encoder.finish()]);
        let (send, recv) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                send.send(result).unwrap()
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        recv.recv().unwrap().unwrap();
        bytemuck::cast_slice(&staging.slice(..).get_mapped_range()).to_vec()
    }
}

// Octahedral encoding of [0, 0.6, 0.8]: y/(|y|+|z|) * 32767 = 14043.
const TILTED_NORMAL_OCT: u32 = 14043 << 16;

// StdVertex is private to the renderer. Supply the documented six-word artifact
// wire layout without exporting it or introducing a second Rust layout struct.
fn fixture_vertices() -> Vec<[u32; 6]> {
    [3.0_f32, 0.0, 0.0, 0.0]
        .iter()
        .enumerate()
        .flat_map(|(index, x)| {
            [
                [-0.4 + x, -0.4, 0.2 + index as f32 * 0.1],
                [0.4 + x, -0.4, 0.2 + index as f32 * 0.1],
                [*x, 0.4, 0.2 + index as f32 * 0.1],
            ]
            .map(|position| {
                [
                    position[0].to_bits(),
                    position[1].to_bits(),
                    position[2].to_bits(),
                    TILTED_NORMAL_OCT,
                    0,
                    0,
                ]
            })
        })
        .collect()
}

fn triangle_material_buffers(gpu: &ArtifactGpu) -> (wgpu::Buffer, wgpu::Buffer) {
    use patinae_render::scene_store::AtomGpu;
    use patinae_render::{ColorLutEntry, RepColorLutEntry};
    let colors = gpu.buffer(
        &[
            ColorLutEntry::default(),
            ColorLutEntry::new([0.2, 0.4, 0.8, 1.0], RepColorLutEntry::inherit_all()),
        ],
        false,
    );
    let atoms = gpu.buffer(
        &[
            AtomGpu {
                vdw: 1.0,
                repr_flags: 0,
                alpha_pack_a: u32::MAX,
                alpha_pack_b: u32::MAX,
            },
            AtomGpu {
                vdw: 1.0,
                repr_flags: 0,
                alpha_pack_a: u32::MAX,
                alpha_pack_b: 127,
            },
        ],
        false,
    );
    (colors, atoms)
}

#[test]
#[ignore = "requires a real GPU; run make test-gpu"]
fn gpu_artifact_triangles_respect_draw_capacity_offsets_normals_and_alpha() {
    use super::layout::ArtifactTriangleParams;
    let gpu = ArtifactGpu::new();
    let vertices = fixture_vertices();
    let source = gpu.buffer(&vertices, false);
    let (colors, atoms) = triangle_material_buffers(&gpu);
    for (label, draw_count, capacity, start, expected_count) in [
        ("draw truncates", 8, 12, 1, 1),
        ("capacity truncates", 12, 8, 1, 1),
        ("nonzero source and destination", 12, 12, 2, 2),
        ("empty", 0, 12, 0, 0),
    ] {
        let output = gpu.buffer(&[0xdead_beef_u32; 5 * 20], false);
        let draw = gpu.buffer(&[draw_count, 1, 0, 0], false);
        let params = gpu.buffer(
            &[ArtifactTriangleParams {
                vertex_capacity: capacity,
                triangle_offset: 1,
                source_triangle_start: start,
                output_triangle_count: 2,
                atom_offset: 1,
                rep_slot: 6,
                transparency: 0.25,
                dispatch_width: 128,
            }],
            true,
        );
        gpu.dispatch(
            &shaders::artifact_triangles(),
            "build_triangles",
            &[
                (0, &source),
                (1, &colors),
                (2, &output),
                (3, &atoms),
                (4, &draw),
                (5, &params),
            ],
        );
        let words = gpu.read::<u32>(&output);
        assert_eq!(&words[..20], &[0xdead_beef; 20], "{label}: prefix guard");
        for i in 0..expected_count {
            let offset = (i + 1) * 20;
            for vertex in 0..3 {
                assert_eq!(
                    &words[offset + vertex * 4..offset + vertex * 4 + 3],
                    &vertices[(start as usize + i) * 3 + vertex][..3],
                    "{label}: positions"
                );
                assert_eq!(
                    words[offset + 16 + vertex],
                    vertices[(start as usize + i) * 3 + vertex][3],
                    "{label}: normal"
                );
            }
            assert_eq!(
                &words[offset + 12..offset + 16],
                &[0.2_f32, 0.4, 0.8, 0.5].map(f32::to_bits),
                "{label}: atom-offset color and alpha override"
            );
        }
        assert!(
            words[(expected_count + 1) * 20..]
                .iter()
                .all(|word| *word == 0xdead_beef),
            "{label}: unwritten tail"
        );
    }
}

#[test]
#[ignore = "requires a real GPU; run make test-gpu"]
fn gpu_artifact_visible_compaction_counts_overflow_and_keeps_guards() {
    let gpu = ArtifactGpu::new();
    let source = gpu.buffer(&fixture_vertices(), false);
    let (colors, atoms) = triangle_material_buffers(&gpu);
    let identity = crate::gpu::RaytraceParams::new(1, 1).view_matrix;
    for (label, draw_count, source_count, start, count, capacity, visible) in [
        ("compaction", 12, 12, 0, 4, 3, 3),
        ("overflow", 12, 12, 0, 4, 1, 3),
        ("draw limit", 6, 12, 0, 4, 3, 1),
        ("source limit", 12, 6, 0, 4, 3, 1),
        ("source slice", 12, 12, 2, 1, 3, 1),
    ] {
        let output = gpu.buffer(&[0xdead_beef_u32; 6 * 20], false);
        let counters = gpu.buffer(&[91_u32, 0, 93], false);
        let draw = gpu.buffer(&[draw_count, 1, 0, 0], false);
        let params = gpu.buffer(
            &[ArtifactVisibleTriangleParams {
                view_matrix: identity,
                proj_matrix: identity,
                source_vertex_count: source_count,
                source_triangle_start: start,
                source_triangle_count: count,
                triangle_offset: 1,
                output_triangle_capacity: capacity,
                atom_offset: 1,
                rep_slot: 6,
                transparency: 0.25,
                dispatch_width: 128,
                counter_index: 1,
                _pad0: 0,
                _pad1: 0,
            }],
            true,
        );
        gpu.dispatch(
            &shaders::artifact_visible_triangles(),
            "build_visible_triangles",
            &[
                (0, &source),
                (1, &colors),
                (2, &output),
                (3, &atoms),
                (4, &draw),
                (5, &counters),
                (6, &params),
            ],
        );
        assert_eq!(
            gpu.read::<u32>(&counters),
            [91, visible, 93],
            "{label}: selected counter"
        );
        let words = gpu.read::<u32>(&output);
        let stored = visible.min(capacity) as usize;
        assert!(
            words[..20].iter().all(|word| *word == 0xdead_beef),
            "{label}: prefix guard"
        );
        assert!(
            words[(stored + 1) * 20..]
                .iter()
                .all(|word| *word == 0xdead_beef),
            "{label}: capacity guard"
        );
        let mut depths: Vec<_> = (0..stored)
            .map(|i| f32::from_bits(words[(i + 1) * 20 + 2]))
            .collect();
        depths.sort_by(f32::total_cmp);
        assert!(
            depths.iter().all(|depth| *depth >= 0.3 && *depth <= 0.5),
            "{label}: only visible geometry"
        );
        assert!(
            depths.windows(2).all(|pair| pair[0] != pair[1]),
            "{label}: compacted triangles are unique"
        );
        if capacity >= visible {
            let expected: Vec<_> = (start..(start + count).min(draw_count.min(source_count) / 3))
                .filter(|index| *index != 0)
                .map(|index| 0.2 + index as f32 * 0.1)
                .collect();
            assert_eq!(depths, expected, "{label}: visible set");
        }
    }
}

#[test]
#[ignore = "requires a real GPU; run make test-gpu"]
fn gpu_artifact_metadata_caps_counts_marks_overflow_and_tiles_dispatch() {
    use super::layout::FinalizeStreamingMetadataParams;
    let gpu = ArtifactGpu::new();
    for (visible, capacity, expected_visible, overflow, dispatch) in [
        (0, 10, 0, 0, [1, 1, 1]),
        (7, 3, 3, 1, [1, 1, 1]),
        (2048, 2048, 2048, 0, [2, 3, 1]),
    ] {
        let metadata = gpu.buffer(
            &[ArtifactPrimitiveMetadata {
                sphere_count: 1,
                cylinder_count: 2,
                capsule_count: 3,
                triangle_count: 999,
                primitive_count: 999,
                triangle_capacity: capacity + 2,
                visible_triangle_count: 999,
                overflow: 99,
            }],
            false,
        );
        let counters = gpu.buffer(&[999_u32, visible], false);
        let params = gpu.buffer(
            &[FinalizeStreamingMetadataParams {
                direct_triangle_count: 2,
                visible_triangle_capacity: capacity,
                counter_index: 1,
                bvh_leaf_size: 4,
                max_workgroups_per_dimension: 2,
                _pad0: 0,
                _pad1: 0,
                _pad2: 0,
            }],
            true,
        );
        let args = gpu.buffer(&[0_u32; 3], false);
        gpu.dispatch(
            &shaders::artifact_finalize_streaming_metadata(),
            "finalize_streaming_metadata",
            &[(0, &metadata), (1, &counters), (2, &params), (3, &args)],
        );
        let actual = gpu.read::<u32>(&metadata);
        assert_eq!(
            actual,
            [
                1,
                2,
                3,
                2 + expected_visible,
                8 + expected_visible,
                capacity + 2,
                expected_visible,
                overflow
            ]
        );
        assert_eq!(gpu.read::<u32>(&args), dispatch);
    }
}

fn compact_triangle(z: f32, alpha: f32) -> ArtifactTriangle {
    let mut words = [0_u32; 20];
    for (index, vertex) in [
        [-1.0, -1.0, z, 0.0],
        [1.0, -1.0, z, 0.0],
        [0.0, 1.0, z, 0.0],
    ]
    .iter()
    .enumerate()
    {
        words[index * 4..index * 4 + 4].copy_from_slice(&vertex.map(f32::to_bits));
    }
    words[12..16].copy_from_slice(&[0.2_f32, 0.4, 0.8, alpha].map(f32::to_bits));
    words[16..19].fill(TILTED_NORMAL_OCT);
    bytemuck::pod_read_unaligned(bytemuck::cast_slice(&words))
}

// The additional entry point only exposes production intersection/shading results.
// It shares every function and binding with the artifact output-buffer shader.
const ARTIFACT_HIT_PROBE: &str = r#"
@compute @workgroup_size(1)
fn probe() {
    let ray = Ray(vec3<f32>(0.0, 0.0, 2.0), vec3<f32>(0.0, 0.0, -1.0));
    let hit = trace_ray(ray, false);
    output_pixels[0] = select(0u, 1u, hit.hit);
    output_pixels[1] = bitcast<u32>(hit.t);
    output_pixels[2] = bitcast<u32>(hit.normal.x);
    output_pixels[3] = bitcast<u32>(hit.normal.y);
    output_pixels[4] = bitcast<u32>(hit.normal.z);
    output_pixels[5] = bitcast<u32>(hit.transparency);
    output_pixels[6] = select(0u, 1u, should_cast_shadow_for_hit(hit));
    let opaque = trace_ray(ray, true);
    output_pixels[7] = select(0u, 1u, opaque.hit);
    output_pixels[8] = bitcast<u32>(opaque.t);
    output_pixels[9] = select(0u, 1u, trace_shadow(ray, 1.5));
    output_pixels[10] = select(0u, 1u, trace_shadow(ray, 0.5));
    let shaded = shade(ray, hit);
    output_pixels[11] = bitcast<u32>(shaded.x);
    output_pixels[12] = bitcast<u32>(shaded.y);
    output_pixels[13] = bitcast<u32>(shaded.z);
}
"#;

#[test]
#[ignore = "requires a real GPU; run make test-gpu"]
fn gpu_artifact_bvh_and_raytrace_obey_metadata_nearest_hit_normals_and_alpha() {
    use super::layout::ArtifactBvhParams;
    use crate::bvh::BvhNode;
    use crate::gpu::uniforms::RaytraceUniforms;
    use bytemuck::Zeroable;
    let gpu = ArtifactGpu::new();
    let spheres = gpu.buffer(&[GpuSphere::zeroed()], false);
    let cylinders = gpu.buffer(&[GpuCylinder::zeroed()], false);
    let capsules = gpu.buffer(&[GpuCapsule::zeroed()], false);
    // Far opaque triangle first: nearest-hit search must not return the first entry.
    let triangles = gpu.buffer(
        &[compact_triangle(0.0, 1.0), compact_triangle(1.0, 0.5)],
        false,
    );
    let params = gpu.buffer(
        &[ArtifactBvhParams {
            leaf_slots: 1,
            leaf_start: 0,
            level_start: 0,
            level_count: 0,
            dispatch_width: 128,
            _pad0: 0,
            _pad1: 0,
        }],
        true,
    );
    for count in [0_u32, 1, 2] {
        let metadata = gpu.buffer(
            &[ArtifactPrimitiveMetadata {
                sphere_count: 0,
                cylinder_count: 0,
                capsule_count: 0,
                triangle_count: count,
                primitive_count: count,
                triangle_capacity: 2,
                visible_triangle_count: 0,
                overflow: 0,
            }],
            false,
        );
        let nodes = gpu.buffer(&[BvhNode::zeroed()], false);
        let indices = gpu.buffer(&[u32::MAX; 4], false);
        gpu.dispatch(
            &shaders::artifact_bvh(),
            "build_leaves",
            &[
                (0, &spheres),
                (1, &cylinders),
                (2, &capsules),
                (3, &triangles),
                (4, &nodes),
                (5, &indices),
                (6, &params),
                (7, &metadata),
            ],
        );
        let node = gpu.read::<BvhNode>(&nodes)[0];
        assert_eq!(node.count, count, "metadata count {count}");
        if count == 0 {
            assert_eq!(node.left_or_first, u32::MAX);
        } else {
            assert_eq!(node.min, [-1.0, -1.0, 0.0]);
            assert_eq!(node.max, [1.0, 1.0, (count - 1) as f32]);
            assert_eq!(
                &gpu.read::<u32>(&indices)[..count as usize],
                &(0..count).map(|i| (2 << 30) | i).collect::<Vec<_>>()
            );
        }
        for (shadows, transparent_shadows) in [(false, false), (true, false), (true, true)] {
            let mut settings = crate::gpu::RaytraceParams::new(3, 2);
            settings.settings.ray_shadow = shadows;
            settings.settings.ray_transparency_shadows = transparent_shadows;
            settings.settings.bg_color = [1.0, 0.0, 0.0, 1.0];
            settings.settings.ray_opaque_background = true;
            let uniforms = gpu.buffer(
                &[RaytraceUniforms::from_counts(&settings, 0, 0, 0, 2, 1)],
                true,
            );
            let output = gpu.buffer(&[0xdead_beef_u32; 16], false);
            let bindings = [
                (0, &uniforms),
                (1, &spheres),
                (2, &cylinders),
                (3, &capsules),
                (4, &triangles),
                (5, &nodes),
                (6, &indices),
                (7, &output),
                (8, &metadata),
            ];
            if count == 0 {
                gpu.dispatch(
                    &shaders::artifact_raytrace_output_buffer(),
                    "main",
                    &bindings,
                );
                let pixels = gpu.read::<u32>(&output);
                assert_eq!(
                    &pixels[..6],
                    &[0xff00_00ff; 6],
                    "empty BVH writes every background pixel"
                );
                assert_eq!(&pixels[6..], &[0xdead_beef; 10], "viewport bounds guard");
                continue;
            }
            let source = shaders::artifact_raytrace_output_buffer() + ARTIFACT_HIT_PROBE;
            gpu.dispatch(&source, "probe", &bindings);
            let words = gpu.read::<u32>(&output);
            assert_eq!(words[0], 1);
            assert_eq!(
                f32::from_bits(words[1]),
                if count == 2 { 1.0 } else { 2.0 },
                "nearest hit honors metadata, not uniform capacity"
            );
            for (actual, expected) in words[2..5]
                .iter()
                .map(|w| f32::from_bits(*w))
                .zip([0.0, 0.6, 0.8])
            {
                assert!(
                    (actual - expected).abs() < 1e-4,
                    "oct normal {actual} != {expected}"
                );
            }
            assert_eq!(f32::from_bits(words[5]), if count == 2 { 0.5 } else { 0.0 });
            assert_eq!(
                words[6],
                u32::from(shadows && (count == 1 || transparent_shadows))
            );
            assert_eq!(words[7], 1);
            assert_eq!(
                f32::from_bits(words[8]),
                2.0,
                "opaque-only traversal skips transparent foreground"
            );
            assert_eq!(words[10], 0, "shadow distance excludes all geometry");
        }
    }
}

#[test]
#[ignore = "requires a real GPU; run make test-gpu"]
fn gpu_artifact_and_standalone_transparent_receivers_gate_shadows() {
    use crate::bvh::BvhNode;
    use crate::gpu::uniforms::RaytraceUniforms;
    use bytemuck::Zeroable;
    let gpu = ArtifactGpu::new();
    // A known hit on a transparent receiver, with an opaque blocker toward the light.
    // Probing shade directly separates shadow receiving from camera occlusion.
    let spheres = gpu.buffer(
        &[GpuSphere::new([0.0, 0.0, 1.5], 0.2, [1.0; 4], 0.0)],
        false,
    );
    let cylinders = gpu.buffer(&[GpuCylinder::zeroed()], false);
    let capsules = gpu.buffer(&[GpuCapsule::zeroed()], false);
    let nodes = gpu.buffer(
        &[BvhNode {
            min: [-0.2, -0.2, 1.3],
            max: [0.2, 0.2, 1.7],
            left_or_first: 0,
            count: 1,
        }],
        false,
    );
    let indices = gpu.buffer(&[0_u32], false);
    let metadata = gpu.buffer(
        &[ArtifactPrimitiveMetadata {
            sphere_count: 1,
            cylinder_count: 0,
            capsule_count: 0,
            triangle_count: 1,
            primitive_count: 2,
            triangle_capacity: 1,
            visible_triangle_count: 0,
            overflow: 0,
        }],
        false,
    );
    let probe = r#"
@compute @workgroup_size(1)
fn shadow_probe() {
    let ray = Ray(vec3<f32>(0.0,0.0,2.0),vec3<f32>(0.0,0.0,-1.0));
    let hit = intersect_triangle(ray, triangles[0]);
    let color = shade(ray, hit);
    output_pixels[0] = bitcast<u32>(color.x);
    output_pixels[1] = bitcast<u32>(color.y);
    output_pixels[2] = bitcast<u32>(color.z);
}
"#;
    for artifact in [true, false] {
        for alpha in [1.0_f32, 0.5] {
            let triangles = if artifact {
                gpu.buffer(&[compact_triangle(1.0, alpha)], false)
            } else {
                gpu.buffer(
                    &[GpuTriangle::new(
                        [-1.0, -1.0, 1.0],
                        [1.0, -1.0, 1.0],
                        [0.0, 1.0, 1.0],
                        [0.0, 0.6, 0.8],
                        [0.0, 0.6, 0.8],
                        [0.0, 0.6, 0.8],
                        [0.2, 0.4, 0.8, alpha],
                        1.0 - alpha,
                    )],
                    false,
                )
            };

            for positional in [false, true] {
                for (shadows, transparent_shadows) in [(false, false), (true, false), (true, true)]
                {
                    let mut settings = crate::gpu::RaytraceParams::new(1, 1);
                    settings.settings.ray_shadow = shadows;
                    settings.settings.ray_transparency_shadows = transparent_shadows;
                    settings.settings.ambient = 0.1;
                    settings.settings.direct = if positional { 0.0 } else { 1.0 };
                    settings.settings.reflect = if positional { 1.0 } else { 0.0 };
                    settings.settings.specular = 0.0;
                    settings.settings.light_count = if positional { 2 } else { 1 };
                    settings.settings.light_dirs[0] = [0.0, 0.0, -1.0, 0.0];
                    let uniforms = gpu.buffer(
                        &[RaytraceUniforms::from_counts(&settings, 1, 0, 0, 1, 1)],
                        true,
                    );
                    let output = gpu.buffer(&[0_u32; 3], false);
                    let mut bindings = vec![
                        (0, &uniforms),
                        (1, &spheres),
                        (2, &cylinders),
                        (3, &capsules),
                        (4, &triangles),
                        (5, &nodes),
                        (6, &indices),
                    ];
                    let source = if artifact {
                        bindings.extend([(7, &output), (8, &metadata)]);
                        shaders::artifact_raytrace_output_buffer() + probe
                    } else {
                        bindings.push((10, &output));
                        format!("{}\n@group(0) @binding(10) var<storage, read_write> output_pixels: array<u32>;\n{probe}", shaders::RAYTRACE)
                    };
                    gpu.dispatch(&source, "shadow_probe", &bindings);
                    let shadowed = shadows && (alpha == 1.0 || transparent_shadows);
                    let brightness = if shadowed { 0.1 } else { 0.9 };
                    for (actual, color) in gpu.read::<f32>(&output).into_iter().zip([0.2, 0.4, 0.8])
                    {
                        assert!((actual-color*brightness).abs() < 1e-4, "alpha={alpha}, positional={positional}, shadows={shadows}, transparent_shadows={transparent_shadows}: {actual}");
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires a real GPU; run make test-gpu"]
fn gpu_artifact_bvh_ignores_stale_inactive_leaves_in_streaming_chunks() {
    use super::layout::ArtifactBvhParams;
    use crate::bvh::BvhNode;
    let gpu = ArtifactGpu::new();
    let live = BvhNode {
        min: [-1.0; 3],
        max: [1.0; 3],
        left_or_first: 0,
        count: 1,
    };
    let stale = BvhNode {
        min: [-100.0; 3],
        max: [100.0; 3],
        left_or_first: 50,
        count: 4,
    };
    let nodes = gpu.buffer(&[stale, stale, stale, live, stale, stale, stale], false);
    let metadata = gpu.buffer(
        &[ArtifactPrimitiveMetadata {
            sphere_count: 1,
            cylinder_count: 0,
            capsule_count: 0,
            triangle_count: 0,
            primitive_count: 1,
            triangle_capacity: 0,
            visible_triangle_count: 0,
            overflow: 0,
        }],
        false,
    );
    for (level_start, level_count) in [(3, 2), (1, 1)] {
        let params = gpu.buffer(
            &[ArtifactBvhParams {
                leaf_slots: 4,
                leaf_start: 3,
                level_start,
                level_count,
                dispatch_width: 128,
                _pad0: 0,
                _pad1: 0,
            }],
            true,
        );
        gpu.dispatch(
            &shaders::artifact_bvh(),
            "build_internal",
            &[(4, &nodes), (6, &params), (7, &metadata)],
        );
    }
    let actual = gpu.read::<BvhNode>(&nodes);
    assert_eq!(actual[0].min, live.min);
    assert_eq!(actual[0].max, live.max);
    assert_eq!(actual[0].left_or_first, live.left_or_first);
    assert_eq!(actual[0].count, 1);
    assert_eq!(actual[2].count, 0);
    assert_eq!(actual[2].left_or_first, u32::MAX);
}
