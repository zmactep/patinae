//! Raytracer procedure bench for setup, GPU dispatch, and readback.
//!
//! Run with:
//!
//! ```sh
//! BENCH_RT_SCENE=surface_2k BENCH_RT_W=512 BENCH_RT_H=512 \
//!   cargo bench -p patinae-bench --bench raytrace_procedure
//! BENCH_RT_SCENE=surface_8k BENCH_RT_W=1024 BENCH_RT_H=1024 BENCH_RT_AA=2 \
//!   cargo bench -p patinae-bench --bench raytrace_procedure
//! ```

use criterion::{
    black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput,
};
use patinae_render::bytes_to_mib;
use raytracer_plugin::bvh::Bvh;
use raytracer_plugin::gpu::{raytrace_profiled, RaytraceContext, RaytraceParams, RaytraceProfile};
use raytracer_plugin::primitive::{GpuCapsule, GpuSphere, GpuTriangle, Primitives};
use raytracer_plugin::settings::RaytraceSettings;

#[derive(Clone, Copy, Debug)]
struct SceneSpec {
    name: &'static str,
    surface_tiles: u32,
    atom_edge: u32,
    include_capsules: bool,
}

impl SceneSpec {
    fn from_env() -> Self {
        match std::env::var("BENCH_RT_SCENE")
            .unwrap_or_else(|_| "surface_2k".to_string())
            .as_str()
        {
            "surface_512" => Self {
                name: "surface_512",
                surface_tiles: 16,
                atom_edge: 0,
                include_capsules: false,
            },
            "surface_8k" => Self {
                name: "surface_8k",
                surface_tiles: 64,
                atom_edge: 0,
                include_capsules: false,
            },
            "surface_32k" => Self {
                name: "surface_32k",
                surface_tiles: 128,
                atom_edge: 0,
                include_capsules: false,
            },
            "mixed_4k" => Self {
                name: "mixed_4k",
                surface_tiles: 32,
                atom_edge: 10,
                include_capsules: true,
            },
            _ => Self {
                name: "surface_2k",
                surface_tiles: 32,
                atom_edge: 0,
                include_capsules: false,
            },
        }
    }
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|&value| value > 0)
        .unwrap_or(default)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| value != "0" && !value.eq_ignore_ascii_case("false") && !value.is_empty())
        .unwrap_or(false)
}

fn request_device() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        ..Default::default()
    }))
    .expect("raytrace benchmark requires a GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bench.raytrace_procedure.device"),
        required_features: wgpu::Features::empty(),
        required_limits: adapter.limits(),
        memory_hints: wgpu::MemoryHints::Performance,
        experimental_features: wgpu::ExperimentalFeatures::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("raytrace benchmark device");

    let info = adapter.get_info();
    let limits = adapter.limits();
    eprintln!(
        "adapter: {} ({:?}); storage_limit={:.2} MiB max_buffer={:.2} MiB",
        info.name,
        info.backend,
        bytes_to_mib(limits.max_storage_buffer_binding_size),
        bytes_to_mib(limits.max_buffer_size)
    );

    (device, queue)
}

fn generate_primitives(spec: SceneSpec) -> Primitives {
    let surface_triangle_count = (spec.surface_tiles * spec.surface_tiles * 2) as usize;
    let atom_count = (spec.atom_edge * spec.atom_edge * spec.atom_edge) as usize;
    let capsule_count = if spec.include_capsules {
        atom_count.saturating_sub(1)
    } else {
        0
    };
    let mut primitives = Primitives {
        spheres: Vec::with_capacity(atom_count),
        cylinders: Vec::new(),
        capsules: Vec::with_capacity(capsule_count),
        triangles: Vec::with_capacity(surface_triangle_count),
    };
    append_surface(&mut primitives, spec.surface_tiles);
    if spec.atom_edge > 0 {
        append_atoms(&mut primitives, spec.atom_edge, spec.include_capsules);
    }
    primitives
}

fn append_surface(primitives: &mut Primitives, tiles: u32) {
    if tiles == 0 {
        return;
    }
    let world_width = 72.0_f32;
    let step = world_width / tiles as f32;
    let origin = -world_width * 0.5;
    let color = [0.2, 0.72, 0.86, 1.0];

    for y in 0..tiles {
        for x in 0..tiles {
            let x0 = origin + x as f32 * step;
            let y0 = origin + y as f32 * step;
            let x1 = x0 + step;
            let y1 = y0 + step;
            let z00 = ripple_z(x0, y0);
            let z10 = ripple_z(x1, y0);
            let z01 = ripple_z(x0, y1);
            let z11 = ripple_z(x1, y1);

            primitives.triangles.push(GpuTriangle::flat(
                [x0, y0, z00],
                [x1, y0, z10],
                [x0, y1, z01],
                color,
                0.0,
            ));
            primitives.triangles.push(GpuTriangle::flat(
                [x1, y0, z10],
                [x1, y1, z11],
                [x0, y1, z01],
                color,
                0.0,
            ));
        }
    }
}

fn append_atoms(primitives: &mut Primitives, edge: u32, include_capsules: bool) {
    let world_width = 38.0_f32;
    let step = world_width / edge.max(1) as f32;
    let origin = -world_width * 0.5;
    let radius = (step * 0.18).max(0.18);
    let mut previous: Option<[f32; 3]> = None;

    for z in 0..edge {
        for y in 0..edge {
            for x in 0..edge {
                let pos = [
                    origin + x as f32 * step,
                    origin + y as f32 * step,
                    8.0 + origin * 0.25 + z as f32 * step * 0.5,
                ];
                let color = [
                    0.75 + (x % 3) as f32 * 0.07,
                    0.32 + (y % 5) as f32 * 0.04,
                    0.44 + (z % 7) as f32 * 0.035,
                    1.0,
                ];
                primitives
                    .spheres
                    .push(GpuSphere::new(pos, radius, color, 0.0));
                if include_capsules {
                    if let Some(start) = previous {
                        primitives.capsules.push(GpuCapsule::new(
                            start,
                            pos,
                            radius * 0.45,
                            color,
                            color,
                            0.0,
                        ));
                    }
                    previous = Some(pos);
                }
            }
        }
    }
}

fn ripple_z(x: f32, y: f32) -> f32 {
    (x * 0.11).sin() * 1.2 + (y * 0.09).cos() * 0.8
}

fn identity() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn translate_z(z: f32) -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, z, 1.0],
    ]
}

fn raytrace_params(width: u32, height: u32, antialias: u32) -> RaytraceParams {
    let camera_z = 80.0;
    let settings = RaytraceSettings {
        ray_shadow: env_flag("BENCH_RT_SHADOWS"),
        ray_trace_mode: env_u32("BENCH_RT_MODE", 0) as i32,
        transparency_mode: env_u32("BENCH_RT_TRANSPARENCY_MODE", 2) as i32,
        ..Default::default()
    };

    RaytraceParams::new(width, height)
        .with_antialias(antialias)
        .with_camera(
            translate_z(-camera_z),
            identity(),
            translate_z(camera_z),
            identity(),
            [0.0, 0.0, camera_z, 1.0],
        )
        .with_settings(settings)
}

fn print_profile(label: &str, profile: &RaytraceProfile) {
    eprintln!(
        "{label}: total_ms={:.3} validate={:.3} pipeline={:.3} upload_buffers={:.3} \
         uniforms={:.3} textures={:.3} bind_group={:.3} encode={:.3} submit={:.3} \
         readback={:.3} downsample={:.3}",
        profile.total_ms,
        profile.validate_ms,
        profile.pipeline_ms,
        profile.upload_buffers_ms,
        profile.uniform_ms,
        profile.textures_ms,
        profile.bind_group_ms,
        profile.encode_ms,
        profile.submit_ms,
        profile.readback_ms,
        profile.downsample_ms,
    );
    eprintln!(
        "{label}: output={}x{} render={}x{} aa={} mode={} primitives={} \
         spheres={} cylinders={} capsules={} triangles={} bvh_nodes={} readback={:.2} MiB",
        profile.output_width,
        profile.output_height,
        profile.render_width,
        profile.render_height,
        profile.antialias,
        profile.ray_trace_mode,
        profile.total_primitives(),
        profile.sphere_count,
        profile.cylinder_count,
        profile.capsule_count,
        profile.triangle_count,
        profile.bvh_node_count,
        bytes_to_mib(profile.readback_bytes),
    );
}

fn bench_raytrace_procedure(c: &mut Criterion) {
    let spec = SceneSpec::from_env();
    let width = env_u32("BENCH_RT_W", 512);
    let height = env_u32("BENCH_RT_H", 512);
    let antialias = env_u32("BENCH_RT_AA", 1);
    eprintln!(
        "BENCH_RT_SCENE={} BENCH_RT_W={} BENCH_RT_H={} BENCH_RT_AA={}",
        spec.name, width, height, antialias
    );

    let primitives = generate_primitives(spec);
    eprintln!(
        "generated: spheres={} cylinders={} capsules={} triangles={} total={}",
        primitives.spheres.len(),
        primitives.cylinders.len(),
        primitives.capsules.len(),
        primitives.triangles.len(),
        primitives.total_count()
    );

    let mut setup_group = c.benchmark_group("raytrace_procedure/setup");
    setup_group.sample_size(10);
    setup_group.bench_with_input(
        BenchmarkId::new("bvh_build", spec.name),
        &primitives,
        |b, primitives| {
            b.iter_batched(
                || primitives.clone(),
                |primitives| black_box(Bvh::build(black_box(&primitives)).expect("BVH build")),
                BatchSize::LargeInput,
            );
        },
    );
    setup_group.finish();

    let bvh = Bvh::build(&primitives).expect("BVH build");
    let params = raytrace_params(width, height, antialias);
    let (device, queue) = request_device();
    let compatibility_sample = raytrace_profiled(&device, &queue, &primitives, &bvh, &params)
        .expect("compatibility profiled raytrace sample");
    black_box(compatibility_sample.image.len());
    print_profile("compat_profile_sample", &compatibility_sample.profile);

    let context_setup_start = std::time::Instant::now();
    let mut sample_context = RaytraceContext::new(&device).expect("raytrace context");
    let context_setup_ms = context_setup_start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("context_setup_ms={context_setup_ms:.3}");
    let cold_context_sample = sample_context
        .raytrace_profiled(&device, &queue, &primitives, &bvh, &params)
        .expect("cold context raytrace sample");
    black_box(cold_context_sample.image.len());
    print_profile("cold_context_profile_sample", &cold_context_sample.profile);
    let warm_context_sample = sample_context
        .raytrace_profiled(&device, &queue, &primitives, &bvh, &params)
        .expect("warm context raytrace sample");
    black_box(warm_context_sample.image.len());
    print_profile("warm_context_profile_sample", &warm_context_sample.profile);

    let mut render_group = c.benchmark_group("raytrace_procedure/full");
    render_group.sample_size(10);
    render_group.throughput(Throughput::Elements(
        warm_context_sample.profile.render_pixels(),
    ));
    render_group.bench_function(
        BenchmarkId::new("raytrace_profiled_wrapper", spec.name),
        |b| {
            b.iter(|| {
                let output = raytrace_profiled(
                    black_box(&device),
                    black_box(&queue),
                    black_box(&primitives),
                    black_box(&bvh),
                    black_box(&params),
                )
                .expect("raytrace");
                black_box((output.image.len(), output.profile.total_ms));
            });
        },
    );
    let mut bench_context = RaytraceContext::new(&device).expect("raytrace bench context");
    render_group.bench_function(
        BenchmarkId::new("raytrace_context_profiled", spec.name),
        |b| {
            b.iter(|| {
                let output = bench_context
                    .raytrace_profiled(
                        black_box(&device),
                        black_box(&queue),
                        black_box(&primitives),
                        black_box(&bvh),
                        black_box(&params),
                    )
                    .expect("raytrace context");
                black_box((output.image.len(), output.profile.total_ms));
            });
        },
    );
    render_group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = bench_raytrace_procedure
}

criterion_main!(benches);
