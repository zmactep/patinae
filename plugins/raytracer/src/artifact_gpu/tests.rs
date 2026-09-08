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

fn scene_atoms_descriptor(id: u64, element_count: u64) -> RenderArtifactBufferDescriptor {
    RenderArtifactBufferDescriptor {
        handle: handle(id),
        role: RenderArtifactBufferRole::SceneAtoms,
        size: element_count * ATOM_STRIDE,
        stride: ATOM_STRIDE,
        element_count,
    }
}

fn assert_shader_has(shader: &str, needle: &str) {
    assert!(
        shader.contains(needle),
        "expected shader to contain `{needle}`"
    );
}

fn assert_shader_lacks(shader: &str, needle: &str) {
    assert!(
        !shader.contains(needle),
        "expected shader not to contain `{needle}`"
    );
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
    let limits = patinae_scene::GpuDeviceLimits {
        max_buffer_size: 4_294_967_296,
        max_storage_buffer_binding_size: 2_147_483_647,
        max_compute_workgroups_per_dimension: 65_535,
        max_compute_invocations_per_workgroup: 256,
        max_compute_workgroup_size_x: 256,
        max_compute_workgroup_size_y: 1,
        max_compute_workgroup_size_z: 1,
        buffer_binding_array: false,
        storage_resource_binding_array: false,
    };
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
    let limits = patinae_scene::GpuDeviceLimits {
        max_buffer_size: 1024,
        max_storage_buffer_binding_size: 512,
        max_compute_workgroups_per_dimension: 65_535,
        max_compute_invocations_per_workgroup: 256,
        max_compute_workgroup_size_x: 256,
        max_compute_workgroup_size_y: 1,
        max_compute_workgroup_size_z: 1,
        buffer_binding_array: false,
        storage_resource_binding_array: false,
    };

    assert!(checked_storage_buffer_size(512, &limits, "test storage").is_ok());
    let err = checked_storage_buffer_size(513, &limits, "test storage").unwrap_err();

    assert!(err.contains("test storage buffer size 513 exceeds GPU storage buffer limit 512"));
}

#[test]
fn planner_selects_streaming_fallback_for_reported_3j3q_surface_capacity() {
    let source_triangles = 143_029_960_u64;
    let source_vertices = source_triangles * 3;
    let limits = patinae_scene::GpuDeviceLimits {
        max_buffer_size: 4_294_967_296,
        max_storage_buffer_binding_size: 2_147_483_647,
        max_compute_workgroups_per_dimension: 65_535,
        max_compute_invocations_per_workgroup: 256,
        max_compute_workgroup_size_x: 256,
        max_compute_workgroup_size_y: 1,
        max_compute_workgroup_size_z: 1,
        buffer_binding_array: false,
        storage_resource_binding_array: false,
    };
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
    let limits = patinae_scene::GpuDeviceLimits {
        max_buffer_size: 4_294_967_296,
        max_storage_buffer_binding_size: 2_147_483_647,
        max_compute_workgroups_per_dimension: 65_535,
        max_compute_invocations_per_workgroup: 256,
        max_compute_workgroup_size_x: 256,
        max_compute_workgroup_size_y: 1,
        max_compute_workgroup_size_z: 1,
        buffer_binding_array: false,
        storage_resource_binding_array: false,
    };
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
    let limits = patinae_scene::GpuDeviceLimits {
        max_buffer_size: 4_294_967_296,
        max_storage_buffer_binding_size: 2_147_483_647,
        max_compute_workgroups_per_dimension: 65_535,
        max_compute_invocations_per_workgroup: 256,
        max_compute_workgroup_size_x: 256,
        max_compute_workgroup_size_y: 1,
        max_compute_workgroup_size_z: 1,
        buffer_binding_array: false,
        storage_resource_binding_array: false,
    };
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
fn raytrace_output_buffer_shader_uses_output_buffer_and_empty_node_guard() {
    let shader = shaders::artifact_raytrace_output_buffer();

    assert_shader_has(&shader, "output_pixels");
    assert_shader_has(
        &shader,
        "global_id.y * u32(uniforms.viewport.x) + global_id.x",
    );
    assert_shader_has(&shader, "node.left_or_first == 0xffffffffu");
    assert_shader_lacks(&shader, "texture_storage_2d");
    assert_shader_lacks(&shader, "textureStore(");
}

#[test]
fn artifact_triangle_shader_uses_gpu_draw_args_without_cpu_decode() {
    let shader = shaders::artifact_triangles();

    assert_shader_has(&shader, "draw_args");
    assert_shader_has(&shader, "min(draw_args[0], params.vertex_capacity)");
    assert_shader_has(&shader, "params.source_triangle_start");
    assert_shader_has(&shader, "scene_atoms");
    assert_shader_has(&shader, "atom.alpha_pack_b");
    assert_shader_has(&shader, "normal_oct: vec4<u32>");
}

#[test]
fn artifact_visible_triangle_shader_uses_gpu_draw_args_and_cursor() {
    let shader = shaders::artifact_visible_triangles();

    assert_shader_has(&shader, "draw_args");
    assert_shader_has(&shader, "min(draw_args[0], params.source_vertex_count)");
    assert_shader_has(&shader, "triangle_index >= params.source_triangle_count");
    assert_shader_has(&shader, "params.source_triangle_start");
    assert_shader_has(
        &shader,
        "atomicAdd(&visible_counts[params.counter_index], 1u)",
    );
    assert_shader_has(&shader, "visible_index >= params.output_triangle_capacity");
}

#[test]
fn artifact_metadata_finalize_shader_caps_visible_count_and_marks_overflow() {
    let shader = shaders::artifact_finalize_streaming_metadata();

    assert_shader_has(&shader, "atomicLoad(&visible_counts[params.counter_index])");
    assert_shader_has(
        &shader,
        "min(visible_count, params.visible_triangle_capacity)",
    );
    assert_shader_has(&shader, "metadata.primitive_count");
    assert_shader_has(&shader, "metadata.overflow");
    assert_shader_has(&shader, "leaf_dispatch_args[0]");
    assert_shader_has(&shader, "workgroups_for_items(metadata.primitive_count)");
}

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
fn artifact_bvh_and_raytrace_shaders_read_gpu_primitive_metadata() {
    let bvh = shaders::artifact_bvh();
    let raytrace = shaders::artifact_raytrace_output_buffer();

    assert_shader_has(&bvh, "var<storage, read> metadata: PrimitiveMetadata");
    assert_shader_has(&bvh, "metadata.primitive_count");
    assert_shader_has(&bvh, "fn active_leaf_count");
    assert_shader_has(&bvh, "fn child_node_or_empty");
    assert_shader_has(&bvh, "child_leaf_start >= active_leaf_count()");
    assert_shader_has(&raytrace, "var<storage, read> metadata: PrimitiveMetadata");
    assert_shader_has(&raytrace, "prim_idx < metadata.triangle_count");
}

#[test]
fn artifact_raytrace_shader_decodes_compact_triangle_normals() {
    let shader = shaders::artifact_raytrace_output_buffer();

    assert_shader_has(&shader, "normal_oct: vec4<u32>");
    assert_shader_has(&shader, "oct_decode(tri.normal_oct.x)");
    assert_shader_has(&shader, "hit.transparency = 1.0 - tri.color.a");
    assert_shader_has(&shader, "trace_ray(current_ray, opaque_only_trace)");
    assert_shader_has(&shader, "opaque_only_trace = true");
}

#[test]
fn raytrace_shaders_gate_transparent_hit_shadows() {
    let artifact = shaders::artifact_raytrace_output_buffer();
    for shader in [shaders::RAYTRACE, artifact.as_str()] {
        assert_shader_has(
            shader,
            "fn should_cast_shadow_for_hit(hit: HitInfo) -> bool",
        );
        assert_shader_has(shader, "uniforms.ray_transparency_shadows != 0u");
        assert_shader_has(shader, "let cast_shadows = should_cast_shadow_for_hit(hit)");
        assert_shader_has(shader, "if cast_shadows && headlight_ndotl > 0.001");
        assert_shader_has(shader, "if i == 0 && cast_shadows && ndotl > 0.001");
        assert_shader_lacks(
            shader,
            "if uniforms.ray_shadow != 0u && headlight_ndotl > 0.001",
        );
    }
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
        device_limits: patinae_scene::GpuDeviceLimits {
            max_buffer_size: 1024,
            max_storage_buffer_binding_size: 1024,
            max_compute_workgroups_per_dimension: 65535,
            max_compute_invocations_per_workgroup: 256,
            max_compute_workgroup_size_x: 256,
            max_compute_workgroup_size_y: 1,
            max_compute_workgroup_size_z: 1,
            buffer_binding_array: false,
            storage_resource_binding_array: false,
        },
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
        device_limits: patinae_scene::GpuDeviceLimits {
            max_buffer_size: 1024,
            max_storage_buffer_binding_size: 1024,
            max_compute_workgroups_per_dimension: 65_535,
            max_compute_invocations_per_workgroup: 256,
            max_compute_workgroup_size_x: 256,
            max_compute_workgroup_size_y: 1,
            max_compute_workgroup_size_z: 1,
            buffer_binding_array: false,
            storage_resource_binding_array: false,
        },
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
        device_limits: patinae_scene::GpuDeviceLimits {
            max_buffer_size: 4096,
            max_storage_buffer_binding_size: 4096,
            max_compute_workgroups_per_dimension: 65_535,
            max_compute_invocations_per_workgroup: 256,
            max_compute_workgroup_size_x: 256,
            max_compute_workgroup_size_y: 1,
            max_compute_workgroup_size_z: 1,
            buffer_binding_array: false,
            storage_resource_binding_array: false,
        },
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
        device_limits: patinae_scene::GpuDeviceLimits {
            max_buffer_size: 4096,
            max_storage_buffer_binding_size: 4096,
            max_compute_workgroups_per_dimension: 65_535,
            max_compute_invocations_per_workgroup: 256,
            max_compute_workgroup_size_x: 256,
            max_compute_workgroup_size_y: 1,
            max_compute_workgroup_size_z: 1,
            buffer_binding_array: false,
            storage_resource_binding_array: false,
        },
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
        device_limits: patinae_scene::GpuDeviceLimits {
            max_buffer_size: mib_to_bytes(128),
            max_storage_buffer_binding_size: mib_to_bytes(128),
            max_compute_workgroups_per_dimension: 65_535,
            max_compute_invocations_per_workgroup: 256,
            max_compute_workgroup_size_x: 256,
            max_compute_workgroup_size_y: 1,
            max_compute_workgroup_size_z: 1,
            buffer_binding_array: false,
            storage_resource_binding_array: false,
        },
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
        device_limits: patinae_scene::GpuDeviceLimits {
            max_buffer_size: 4096,
            max_storage_buffer_binding_size: 4096,
            max_compute_workgroups_per_dimension: 65_535,
            max_compute_invocations_per_workgroup: 256,
            max_compute_workgroup_size_x: 256,
            max_compute_workgroup_size_y: 1,
            max_compute_workgroup_size_z: 1,
            buffer_binding_array: false,
            storage_resource_binding_array: false,
        },
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
        device_limits: patinae_scene::GpuDeviceLimits {
            max_buffer_size: 4096,
            max_storage_buffer_binding_size: 4096,
            max_compute_workgroups_per_dimension: 65_535,
            max_compute_invocations_per_workgroup: 256,
            max_compute_workgroup_size_x: 256,
            max_compute_workgroup_size_y: 1,
            max_compute_workgroup_size_z: 1,
            buffer_binding_array: false,
            storage_resource_binding_array: false,
        },
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
        device_limits: patinae_scene::GpuDeviceLimits {
            max_buffer_size: 4096,
            max_storage_buffer_binding_size: 4096,
            max_compute_workgroups_per_dimension: 65_535,
            max_compute_invocations_per_workgroup: 256,
            max_compute_workgroup_size_x: 256,
            max_compute_workgroup_size_y: 1,
            max_compute_workgroup_size_z: 1,
            buffer_binding_array: false,
            storage_resource_binding_array: false,
        },
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
