//! Exercise production stick shaders against an independent f64 silhouette.

use std::sync::Arc;

use patinae_mol::{
    Atom, AtomIndex, BondOrder, CoordSet, DirtyFlags, Element, ObjectMolecule, RepMask,
};
use patinae_settings::{ResolvedSettings, Settings};

use crate::capture::capture_rgba;
use crate::{
    FrameUniforms, ObjectId, PickingMode, RenderAtomColors, RenderConfig, RenderInput,
    RenderMemoryPolicy, RenderObjectInput, RenderState, SceneLod, IDENTITY_TRANSFORM,
};

const SIZE: u32 = 256;
const HALF: f64 = 3.0;

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn add_scaled(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + b[i] * t)
}

// Minimize distance from a forward ray to a finite segment. This does not
// duplicate the shader's quadratic intersection/discriminant calculation.
fn ray_segment_distance(ro: [f64; 3], rd: [f64; 3], pa: [f64; 3], pb: [f64; 3]) -> f64 {
    let t = closest_ray_t(ro, rd, pa, pb);
    point_segment_distance(add_scaled(ro, rd, t), pa, pb)
}

fn closest_ray_t(ro: [f64; 3], rd: [f64; 3], pa: [f64; 3], pb: [f64; 3]) -> f64 {
    let ba = sub(pb, pa);
    let oa = sub(ro, pa);
    let length2 = dot(ba, ba);
    let axis_ray = dot(ba, rd);
    let denom = length2 - axis_ray * axis_ray;
    let u = if denom > 1e-12 {
        ((dot(ba, oa) - axis_ray * dot(rd, oa)) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let point = add_scaled(pa, ba, u);
    dot(sub(point, ro), rd).max(0.0)
}

fn point_segment_distance(point: [f64; 3], pa: [f64; 3], pb: [f64; 3]) -> f64 {
    let ba = sub(pb, pa);
    let u = (dot(sub(point, pa), ba) / dot(ba, ba)).clamp(0.0, 1.0);
    let offset = sub(point, add_scaled(pa, ba, u));
    dot(offset, offset).sqrt()
}

fn reference_entry(
    ro: [f64; 3],
    rd: [f64; 3],
    pa: [f64; 3],
    pb: [f64; 3],
    radius: f64,
) -> [f64; 3] {
    let mut lo = 0.0;
    let mut hi = closest_ray_t(ro, rd, pa, pb);
    // Distance to a convex capsule decreases up to the closest point. Bisect
    // its signed distance instead of reimplementing the shader's solver.
    for _ in 0..48 {
        let mid = (lo + hi) * 0.5;
        if point_segment_distance(add_scaled(ro, rd, mid), pa, pb) > radius {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    add_scaled(ro, rd, hi)
}

fn depth_pipeline(device: &wgpu::Device) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stick-test.depth-read"),
        source: wgpu::ShaderSource::Wgsl(
            r"
@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var<storage, read_write> output: array<f32>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(depth);
    if all(id.xy < size) { output[id.y * size.x + id.x] = textureLoad(depth, vec2<i32>(id.xy), 0); }
}"
            .into(),
        ),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("stick-test.depth-read"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn read_depth(
    state: &RenderState,
    pipeline: &wgpu::ComputePipeline,
    view: &wgpu::TextureView,
    size: u32,
) -> Vec<f32> {
    let device = &state.ctx.device;
    let bytes = u64::from(size) * u64::from(size) * 4;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, bytes);
    state.ctx.queue.submit([encoder.finish()]);
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .unwrap();
    let values =
        bytemuck::cast_slice::<u8, f32>(&staging.slice(..).get_mapped_range().unwrap()).to_vec();
    staging.unmap();
    values
}

fn frame(distance: f32, perspective: bool) -> FrameUniforms {
    let mut f = FrameUniforms::default();
    f.view[3][2] = -distance;
    f.view_inv[3][2] = distance;
    let near = 1.0;
    let far = distance + 40.0;
    let scale = if perspective {
        distance / HALF as f32
    } else {
        1.0 / HALF as f32
    };
    if perspective {
        let a = far / (near - far);
        let b = near * a;
        f.proj = [
            [scale, 0.0, 0.0, 0.0],
            [0.0, scale, 0.0, 0.0],
            [0.0, 0.0, a, -1.0],
            [0.0, 0.0, b, 0.0],
        ];
        f.proj_inv = [
            [1.0 / scale, 0.0, 0.0, 0.0],
            [0.0, 1.0 / scale, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0 / b],
            [0.0, 0.0, -1.0, a / b],
        ];
    } else {
        f.proj = [
            [scale, 0.0, 0.0, 0.0],
            [0.0, scale, 0.0, 0.0],
            [0.0, 0.0, 1.0 / (near - far), 0.0],
            [0.0, 0.0, near / (near - far), 1.0],
        ];
        f.proj_inv = [
            [1.0 / scale, 0.0, 0.0, 0.0],
            [0.0, 1.0 / scale, 0.0, 0.0],
            [0.0, 0.0, near - far, 0.0],
            [0.0, 0.0, -near, 1.0],
        ];
    }
    f.view_proj = f.proj;
    for i in 0..4 {
        f.view_proj[3][i] -= distance * f.proj[2][i];
    }
    f.clip = [near, far, 0.5, far];
    f.light_intensity = [1.0, 0.0, 0.0, 0.0];
    f.light_counts[0] = 1.0;
    f
}

#[test]
#[ignore = "requires a real GPU; checks stick precision, projection, and picking"]
fn gpu_sticks_keep_solid_silhouettes_at_constant_scale() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU required");
    eprintln!("Stick precision adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: crate::required_device_limits(&adapter.limits()).unwrap(),
        ..Default::default()
    }))
    .unwrap();
    let mut state = RenderState::with_config(
        Arc::new(device),
        Arc::new(queue),
        wgpu::TextureFormat::Rgba8Unorm,
        (SIZE, SIZE),
        RenderConfig {
            picking: PickingMode::FullRecord,
            memory: RenderMemoryPolicy::performance(),
            ..Default::default()
        },
    );
    state.set_clear_color([0.0, 0.0, 0.0], false);
    state.set_fxaa(false);
    let depth_reader = depth_pipeline(&state.ctx.device);
    let mut molecule = ObjectMolecule::new("stick-precision");
    for name in ["C1", "C2"] {
        let mut atom = Atom::new(name, Element::Carbon);
        atom.repr.visible_reps = RepMask::STICKS;
        molecule.add_atom(atom);
    }
    molecule
        .add_bond(AtomIndex(0), AtomIndex(1), BondOrder::Single)
        .unwrap();
    let colors = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]];
    let mut settings = ResolvedSettings::resolve(&Settings::default(), None);
    for perspective in [false, true] {
        for distance in [5.0, 50.0, 100.0, 232.0, 500.0, 1000.0] {
            for (pa, pb) in [
                ([-1.4, -0.4, 0.0], [1.4, 0.4, 0.0]),
                ([0.7, -1.2, -2.0], [1.8, 1.2, 2.0]),
                ([1.2, 0.5, -2.0], [1.2, 0.5, 2.0]),
            ] {
                for radius in [0.08, 0.19, 0.4] {
                    settings.stick.radius = radius;
                    let coords =
                        CoordSet::from_coords(pa.into_iter().chain(pb).map(|v| v as f32).collect());
                    let object = RenderObjectInput {
                        object_id: ObjectId(1),
                        molecule: &molecule,
                        coord_set: &coords,
                        instances: None,
                        transform: IDENTITY_TRANSFORM,
                        visible_reps: RepMask::STICKS,
                        draw_reps: RepMask::STICKS,
                        object_settings: None,
                        colors: RenderAtomColors::Separate {
                            base: &colors,
                            reps: &[],
                        },
                        atom_markers: &[],
                        recent_atom_markers: None,
                        marker_updates: &[],
                        has_markers: false,
                        lod: SceneLod::Auto,
                        dirty: DirtyFlags::ALL,
                    };
                    let objects = [object];
                    let input = RenderInput {
                        objects: &objects,
                        maps: &[],
                        strokes: &[],
                        settings: &settings,
                        lod: SceneLod::Auto,
                    };
                    let pixels = capture_rgba(
                        &mut state,
                        SIZE,
                        SIZE,
                        &frame(distance, perspective),
                        &input,
                    )
                    .unwrap();
                    let depths = read_depth(&state, &depth_reader, &state.targets.depth, SIZE);
                    let pa_view = add_scaled(pa, [0.0, 0.0, -1.0], distance as f64);
                    let pb_view = add_scaled(pb, [0.0, 0.0, -1.0], distance as f64);
                    let mut misses = 0;
                    let mut extras = 0;
                    let mut interior = 0;
                    for y in 0..SIZE {
                        for x in 0..SIZE {
                            let xy = [
                                (2.0 * (x as f64 + 0.5) / SIZE as f64 - 1.0) * HALF,
                                (1.0 - 2.0 * (y as f64 + 0.5) / SIZE as f64) * HALF,
                            ];
                            let (ro, rd) = if perspective {
                                let v = [xy[0], xy[1], -(distance as f64)];
                                let length = dot(v, v).sqrt();
                                ([0.0; 3], v.map(|c| c / length))
                            } else {
                                ([xy[0], xy[1], 0.0], [0.0, 0.0, -1.0])
                            };
                            let separation = ray_segment_distance(ro, rd, pa_view, pb_view);
                            // Exclude one screen pixel around the analytic boundary.
                            let margin = 2.0 * HALF / SIZE as f64 * (1.0 + 2.0 / distance as f64);
                            let visible = pixels[((y * SIZE + x) * 4 + 3) as usize] > 0;
                            if separation < radius as f64 - margin {
                                interior += 1;
                                misses += usize::from(!visible);
                                if x % 8 == 0 && y % 8 == 0 {
                                    let point =
                                        reference_entry(ro, rd, pa_view, pb_view, radius as f64);
                                    let proj = frame(distance, perspective).proj;
                                    let expected = (proj[2][2] as f64 * point[2]
                                        + proj[3][2] as f64)
                                        / (proj[2][3] as f64 * point[2] + proj[3][3] as f64);
                                    let actual = depths[(y * SIZE + x) as usize] as f64;
                                    // Bound proxy interpolation error by 1/16 screen pixel,
                                    // then add eight f32 epsilons for depth projection.
                                    // Use the local analytic depth slope, not a tolerance
                                    // fitted to one camera distance or one failed sample.
                                    let mut tolerance = 8.0 * f32::EPSILON as f64;
                                    let delta = 2.0 * HALF / SIZE as f64 / 16.0;
                                    for dx in [-delta, delta] {
                                        for dy in [-delta, delta] {
                                            let (jro, jrd) = if perspective {
                                                let v =
                                                    [xy[0] + dx, xy[1] + dy, -(distance as f64)];
                                                let norm = dot(v, v).sqrt();
                                                ([0.0; 3], v.map(|c| c / norm))
                                            } else {
                                                ([xy[0] + dx, xy[1] + dy, 0.0], rd)
                                            };
                                            let p = reference_entry(
                                                jro,
                                                jrd,
                                                pa_view,
                                                pb_view,
                                                radius as f64,
                                            );
                                            let z = (proj[2][2] as f64 * p[2] + proj[3][2] as f64)
                                                / (proj[2][3] as f64 * p[2] + proj[3][3] as f64);
                                            tolerance = tolerance.max(
                                                (z - expected).abs() + 8.0 * f32::EPSILON as f64,
                                            );
                                        }
                                    }
                                    assert!((actual-expected).abs() < tolerance,"depth {actual}/{expected}, tolerance={tolerance}, d={distance}, perspective={perspective}");
                                }
                            }
                            if separation > radius as f64 + margin {
                                extras += usize::from(visible);
                            }
                        }
                    }
                    assert!(interior > 0);
                    assert_eq!((misses,extras),(0,0),"projection={perspective}, d={distance}, radius={radius}, ends={pa:?}/{pb:?}");
                    // Pick a known interior location; half-resolution picking uses
                    // the same geometry but its own raster grid.
                    let mid = std::array::from_fn::<_, 3, _>(|i| (pa[i] + pb[i]) * 0.5);
                    let x = ((mid[0] / HALF + 1.0) * 0.5 * SIZE as f64) as u32;
                    let y = ((1.0 - mid[1] / HALF) * 0.5 * SIZE as f64) as u32;
                    assert_eq!(
                        state.pick(x, y).map(|h| h.object_id),
                        Some(ObjectId(1)),
                        "unpickable d={distance}"
                    );
                    if pa[2] == 0.0 && pb[2] == 0.0 {
                        for (u, atom_id) in [(0.25, 0), (0.75, 1)] {
                            let p = add_scaled(pa, sub(pb, pa), u);
                            let x = ((p[0] / HALF + 1.0) * 0.5 * SIZE as f64) as u32;
                            let y = ((1.0 - p[1] / HALF) * 0.5 * SIZE as f64) as u32;
                            assert_eq!(state.pick(x, y).map(|h| h.atom_id), Some(atom_id));
                            let pixel = &pixels[((y * SIZE + x) * 4) as usize..][..4];
                            assert!(
                                pixel[atom_id as usize] > 240,
                                "wrong half-bond color: {pixel:?}"
                            );
                            assert_eq!(pixel[1 - atom_id as usize], 0);
                        }
                    }
                    // Exercise WBOIT with the same geometry, leaving depth clear.
                    if distance == 232.0 && radius == 0.19 {
                        let mut translucent = settings.clone();
                        translucent.stick.transparency = 0.5;
                        let alpha_input = RenderInput {
                            settings: &translucent,
                            ..input
                        };
                        let alpha_pixels = capture_rgba(
                            &mut state,
                            SIZE,
                            SIZE,
                            &frame(distance, perspective),
                            &alpha_input,
                        )
                        .unwrap();
                        for (opaque, alpha) in pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .zip(alpha_pixels.as_chunks::<4>().0)
                        {
                            assert_eq!(opaque[3] > 0, alpha[3] > 0, "WBOIT silhouette differs");
                        }
                        let depths = read_depth(&state, &depth_reader, &state.targets.depth, SIZE);
                        assert!(
                            depths.iter().all(|&z| z == 1.0),
                            "transparent stick wrote opaque depth"
                        );
                    }
                }
            }
        }
    }

    // The first in-frustum surface may be the exit root when near clipping
    // cuts through a cylinder or cap, including an eye-plane crossing proxy.
    for perspective in [false, true] {
        for (pa, pb, visible) in [
            ([-1.0, 0.0, 4.0], [1.0, 0.0, 4.0], true),
            ([-1.0, 0.0, 4.5], [1.0, 0.0, 4.5], false),
            ([0.0, 0.0, 3.5], [0.0, 0.0, 5.5], true),
            ([-1.0, 0.0, -41.0], [1.0, 0.0, -41.0], false),
        ] {
            settings.stick.radius = 0.19;
            let coords = CoordSet::from_coords(pa.into_iter().chain(pb).collect());
            let objects = [RenderObjectInput {
                object_id: ObjectId(1),
                molecule: &molecule,
                coord_set: &coords,
                instances: None,
                transform: IDENTITY_TRANSFORM,
                visible_reps: RepMask::STICKS,
                draw_reps: RepMask::STICKS,
                object_settings: None,
                colors: RenderAtomColors::Separate {
                    base: &colors,
                    reps: &[],
                },
                atom_markers: &[],
                recent_atom_markers: None,
                marker_updates: &[],
                has_markers: false,
                lod: SceneLod::Auto,
                dirty: DirtyFlags::ALL,
            }];
            let input = RenderInput {
                objects: &objects,
                maps: &[],
                strokes: &[],
                settings: &settings,
                lod: SceneLod::Auto,
            };
            let pixels =
                capture_rgba(&mut state, SIZE, SIZE, &frame(5.0, perspective), &input).unwrap();
            let center = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
            assert_eq!(
                pixels[center + 3] > 0,
                visible,
                "clipping {pa:?}/{pb:?}, perspective={perspective}"
            );
            assert_eq!(state.pick(SIZE / 2, SIZE / 2).is_some(), visible);
            let depths = read_depth(&state, &depth_reader, &state.targets.depth, SIZE);
            let depth = depths[center / 4];
            assert!(depth.is_finite() && (0.0..=1.0).contains(&depth));
            if visible {
                assert!(depth < 1.0);
            }
        }
    }
}
