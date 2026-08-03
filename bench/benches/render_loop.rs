//! patinae-render frame-loop bench. Measures `sync + render + submit`
//! latency on a real structure loaded via `patinae-io`. Pass the structure
//! file with `BENCH_FILE`:
//!
//! ```sh
//! BENCH_FILE="$TEST_STRUCTURES_DIR/1fsd.cif" BENCH_SCENARIO=cartoon_only cargo bench -p patinae-bench --bench render_loop
//! BENCH_FILE="$TEST_STRUCTURES_DIR/7KP3-assembly1.cif.gz" cargo bench -p patinae-bench --bench render_loop
//! BENCH_FILE="$TEST_STRUCTURES_DIR/3J3Q.cif" BENCH_SCENARIO=cartoon_only cargo bench -p patinae-bench --bench render_loop
//! BENCH_FILE="$TEST_STRUCTURES_DIR/3J3Q.cif.gz" BENCH_SCENARIO=skripkin_silhouettes cargo bench -p patinae-bench --bench render_loop
//! BENCH_FILE="$TEST_STRUCTURES_DIR/3J3Q.cif.gz" BENCH_SCENARIO=cartoon_toggle_large cargo bench -p patinae-bench --bench render_loop
//! BENCH_SYNTHETIC_WATER_ATOMS=100000 BENCH_REPS=dots cargo bench -p patinae-bench --bench render_loop
//! ```
//!
//! After the criterion measurement, prints a one-line summary of the
//! median `FrameStats` (CPU timings + per-GPU-pass ms) so the reader can
//! see which slot dominates.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use lin_alg::f32::Vec3;
use patinae_color::{ColorResolver, NamedPalette, ThemedPalette};
use patinae_mol::{
    Atom, AtomIndex, AtomResidue, BondOrder, CoordSet, DirtyFlags, Element, ObjectMolecule, RepMask,
};
use patinae_render::scene_store::marker::{MARKER_HOVER, MARKER_SELECTED};
use patinae_render::{
    bytes_to_gib, required_limits_for_memory_policy, select_render_memory_policy, ObjectId,
    PickingMode, RenderConfig, RenderInput, RenderMemoryPolicy, RenderMemoryProfile,
    RenderMemorySelectionInput, RenderObjectInput, RenderState, RenderSyncTimings, SceneLod,
    IDENTITY_TRANSFORM,
};
use patinae_settings::{ResolvedSettings, Settings};

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("bench crate inside project root")
        .to_path_buf()
}

fn resolve_project_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        project_root().join(path)
    }
}

fn bench_file_path() -> PathBuf {
    if let Some(path) = std::env::var_os("BENCH_FILE") {
        return resolve_project_path(PathBuf::from(path));
    }
    let Some(root) = std::env::var_os("TEST_STRUCTURES_DIR") else {
        panic!(
            "BENCH_FILE or TEST_STRUCTURES_DIR environment variable is required.\n\
             Usage: BENCH_FILE=path/to/structure.cif.gz cargo bench -p patinae-bench --bench render_loop"
        );
    };
    resolve_project_path(PathBuf::from(root)).join("1fsd.cif")
}

fn load_molecule() -> ObjectMolecule {
    let path = bench_file_path();
    assert!(path.exists(), "bench file not found: {}", path.display());
    patinae_io::cif::read_cif(&path).expect("failed to parse CIF")
}

fn synthetic_water_atom_count() -> Option<usize> {
    std::env::var("BENCH_SYNTHETIC_WATER_ATOMS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
}

fn load_bench_molecule() -> ObjectMolecule {
    if let Some(atom_count) = synthetic_water_atom_count() {
        eprintln!("BENCH_SYNTHETIC_WATER_ATOMS={atom_count}");
        return synthetic_water_molecule(atom_count);
    }
    load_molecule()
}

fn synthetic_water_molecule(atom_count: usize) -> ObjectMolecule {
    let water_count = atom_count.div_ceil(3);
    let mut mol = ObjectMolecule::with_capacity("synthetic_water", atom_count, water_count * 2);
    let mut coords = Vec::with_capacity(atom_count);
    let grid = (water_count as f32).cbrt().ceil().max(1.0) as usize;
    let spacing = 3.0_f32;

    for water_idx in 0..water_count {
        if mol.atom_count() >= atom_count {
            break;
        }
        let ix = water_idx % grid;
        let iy = (water_idx / grid) % grid;
        let iz = water_idx / (grid * grid);
        let center = Vec3::new(
            ix as f32 * spacing,
            iy as f32 * spacing,
            iz as f32 * spacing,
        );
        let resv = i32::try_from(water_idx + 1).unwrap_or(i32::MAX);
        let residue = Arc::new(AtomResidue::from_parts("W", "HOH", resv, ' ', ""));

        let mut oxygen = Atom::new("O", Element::Oxygen);
        oxygen.residue = residue.clone();
        let oxygen_idx = mol.add_atom(oxygen);
        coords.push(center);

        if mol.atom_count() >= atom_count {
            continue;
        }
        let mut h1 = Atom::new("H1", Element::Hydrogen);
        h1.residue = residue.clone();
        let h1_idx = mol.add_atom(h1);
        coords.push(center + Vec3::new(0.96, 0.0, 0.0));
        let _ = mol.add_bond(oxygen_idx, h1_idx, BondOrder::Single);

        if mol.atom_count() >= atom_count {
            continue;
        }
        let mut h2 = Atom::new("H2", Element::Hydrogen);
        h2.residue = residue;
        let h2_idx = mol.add_atom(h2);
        coords.push(center + Vec3::new(-0.24, 0.93, 0.0));
        let _ = mol.add_bond(oxygen_idx, h2_idx, BondOrder::Single);
    }

    mol.add_coord_set(CoordSet::from_vec3(&coords));
    mol
}

fn default_settings() -> ResolvedSettings {
    ResolvedSettings::resolve(&Settings::default(), None)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| v != "0" && !v.eq_ignore_ascii_case("false") && !v.is_empty())
        .unwrap_or(false)
}

fn force_visible_reps(mol: &mut ObjectMolecule, visible: RepMask) {
    let n_atoms = mol.atom_count();
    for idx in 0..n_atoms {
        if let Some(atom) = mol.atoms_mut().nth(idx) {
            atom.repr.visible_reps = visible;
        }
    }
}

fn checksum_bytes<'a>(rows: impl Iterator<Item = &'a [u8]>) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET;
    for row in rows {
        for &byte in row {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

fn rendered_frame_checksum(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    state: &mut RenderState,
    target_texture: &wgpu::Texture,
    target_view: &wgpu::TextureView,
    width: u32,
    height: u32,
) -> u64 {
    let bytes_per_pixel = 4u32;
    let row_bytes = width.saturating_mul(bytes_per_pixel);
    let padded_row_bytes =
        row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer_size = u64::from(padded_row_bytes) * u64::from(height.max(1));
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bench.frame_checksum.readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("bench.frame_checksum.encoder"),
    });
    state.render(target_view, &mut encoder);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row_bytes),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("device poll");
    rx.recv()
        .expect("frame checksum map callback")
        .expect("frame checksum map");

    let data = slice.get_mapped_range();
    let hash = checksum_bytes((0..height as usize).map(|row| {
        let start = row * padded_row_bytes as usize;
        let end = start + row_bytes as usize;
        &data[start..end]
    }));
    drop(data);
    staging.unmap();
    hash
}

#[derive(Debug, Clone, Copy)]
struct BenchScenario {
    name: &'static str,
    reps: &'static str,
    markers: &'static str,
    shadows: bool,
    skripkin: bool,
    silhouettes: bool,
    force_atlas_rebuild: bool,
}

fn bench_scenario() -> BenchScenario {
    match std::env::var("BENCH_SCENARIO")
        .unwrap_or_else(|_| "classic_small".to_string())
        .as_str()
    {
        "cartoon_only" => BenchScenario {
            name: "cartoon_only",
            reps: "cartoon",
            markers: "none",
            shadows: false,
            skripkin: false,
            silhouettes: false,
            force_atlas_rebuild: false,
        },
        "cartoon_toggle_large" => BenchScenario {
            name: "cartoon_toggle_large",
            reps: "cartoon",
            markers: "none",
            shadows: false,
            skripkin: false,
            silhouettes: false,
            force_atlas_rebuild: false,
        },
        "classic_large" => BenchScenario {
            name: "classic_large",
            reps: "cartoon,sticks,spheres",
            markers: "none",
            shadows: false,
            skripkin: false,
            silhouettes: false,
            force_atlas_rebuild: false,
        },
        "full_shadows" => BenchScenario {
            name: "full_shadows",
            reps: "cartoon,sticks,spheres",
            markers: "none",
            shadows: true,
            skripkin: false,
            silhouettes: false,
            force_atlas_rebuild: false,
        },
        "skripkin_cached" => BenchScenario {
            name: "skripkin_cached",
            reps: "cartoon,sticks,spheres",
            markers: "none",
            shadows: false,
            skripkin: true,
            silhouettes: false,
            force_atlas_rebuild: false,
        },
        "skripkin_forced_rebuild" => BenchScenario {
            name: "skripkin_forced_rebuild",
            reps: "cartoon,sticks,spheres",
            markers: "none",
            shadows: false,
            skripkin: true,
            silhouettes: false,
            force_atlas_rebuild: true,
        },
        "skripkin_silhouettes" => BenchScenario {
            name: "skripkin_silhouettes",
            reps: "cartoon,sticks,spheres",
            markers: "none",
            shadows: false,
            skripkin: true,
            silhouettes: true,
            force_atlas_rebuild: false,
        },
        "sticks_heavy" => BenchScenario {
            name: "sticks_heavy",
            reps: "sticks",
            markers: "none",
            shadows: false,
            skripkin: false,
            silhouettes: false,
            force_atlas_rebuild: false,
        },
        "marking" => BenchScenario {
            name: "marking",
            reps: "cartoon,sticks,spheres",
            markers: "selected",
            shadows: false,
            skripkin: false,
            silhouettes: false,
            force_atlas_rebuild: false,
        },
        "surface_cartoon_large" => BenchScenario {
            name: "surface_cartoon_large",
            reps: "surface,cartoon,sticks",
            markers: "none",
            shadows: false,
            skripkin: false,
            silhouettes: false,
            force_atlas_rebuild: false,
        },
        _ => BenchScenario {
            name: "classic_small",
            reps: "cartoon,sticks,spheres",
            markers: "none",
            shadows: false,
            skripkin: false,
            silhouettes: false,
            force_atlas_rebuild: false,
        },
    }
}

/// Bench viewport — override via `BENCH_W` / `BENCH_H` env vars to test
/// fillrate scaling (e.g. retina @ 3024×1964 vs the default 1024×1024).
fn bench_dims() -> (u32, u32) {
    let w = std::env::var("BENCH_W")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1024);
    let h = std::env::var("BENCH_H")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1024);
    (w, h)
}

fn bench_memory_override() -> Option<RenderMemoryProfile> {
    std::env::var("BENCH_MEMORY_PROFILE")
        .ok()
        .and_then(|value| match value.parse::<RenderMemoryProfile>() {
            Ok(profile) => Some(profile),
            Err(err) => {
                eprintln!("ignoring invalid BENCH_MEMORY_PROFILE={value:?}: {err}");
                None
            }
        })
}

fn request_device() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>, RenderMemoryPolicy) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .expect("no GPU adapter");
    // Bench wants timestamps. If the adapter doesn't expose them, fall
    // back to no features (CPU-only stats will still work).
    let wanted = wgpu::Features::TIMESTAMP_QUERY;
    let features = adapter.features() & wanted;
    let adapter_info = adapter.get_info();
    let adapter_limits = adapter.limits();
    let memory_policy = select_render_memory_policy(
        RenderMemorySelectionInput::from_wgpu(&adapter_info, &adapter_limits, false),
        bench_memory_override(),
    );
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bench.render_loop.device"),
        required_features: features,
        required_limits: required_limits_for_memory_policy(&adapter_limits, memory_policy),
        memory_hints: memory_policy.wgpu_memory_hints(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("device");
    let lim = adapter_limits;
    eprintln!(
        "adapter: {}, TIMESTAMP_QUERY: {}",
        adapter_info.name,
        features.contains(wgpu::Features::TIMESTAMP_QUERY)
    );
    eprintln!(
        "BENCH_MEMORY_PROFILE={} budget={:?}",
        memory_policy.profile, memory_policy.budget_bytes
    );
    eprintln!(
        "limits: max_storage_buffer_binding_size={} ({:.2} GiB)  max_buffer_size={} ({:.2} GiB)  \
         max_compute_workgroups_per_dim={}",
        lim.max_storage_buffer_binding_size,
        bytes_to_gib(lim.max_storage_buffer_binding_size),
        lim.max_buffer_size,
        bytes_to_gib(lim.max_buffer_size),
        lim.max_compute_workgroups_per_dimension,
    );
    (Arc::new(device), Arc::new(queue), memory_policy)
}

fn build_orthographic(half: f32) -> [[f32; 4]; 4] {
    let r = half;
    [
        [1.0 / r, 0.0, 0.0, 0.0],
        [0.0, 1.0 / r, 0.0, 0.0],
        [0.0, 0.0, -1.0 / 1000.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn mat4_mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0_f32; 4]; 4];
    for col in 0..4 {
        for row in 0..4 {
            out[col][row] = a[0][row] * b[col][0]
                + a[1][row] * b[col][1]
                + a[2][row] * b[col][2]
                + a[3][row] * b[col][3];
        }
    }
    out
}

fn rotation_view(angle: f32, center: [f32; 3], distance: f32) -> [[f32; 4]; 4] {
    let c = angle.cos();
    let s = angle.sin();
    [
        [c, 0.0, s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [-s, 0.0, c, 0.0],
        [-center[0], -center[1], -center[2] - distance, 1.0],
    ]
}

fn rigid_view_inverse(view: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let t = [view[3][0], view[3][1], view[3][2]];
    let inv_t = [
        -(view[0][0] * t[0] + view[0][1] * t[1] + view[0][2] * t[2]),
        -(view[1][0] * t[0] + view[1][1] * t[1] + view[1][2] * t[2]),
        -(view[2][0] * t[0] + view[2][1] * t[1] + view[2][2] * t[2]),
    ];
    [
        [view[0][0], view[1][0], view[2][0], 0.0],
        [view[0][1], view[1][1], view[2][1], 0.0],
        [view[0][2], view[1][2], view[2][2], 0.0],
        [inv_t[0], inv_t[1], inv_t[2], 1.0],
    ]
}

fn print_histogram(name: &str, samples: &mut [f32]) {
    if samples.is_empty() {
        eprintln!("(no {name} samples - pass skipped or adapter lacks TIMESTAMP_QUERY)");
        return;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = samples.len();
    let p = |q: f32| samples[((n as f32 * q) as usize).min(n - 1)];
    eprintln!("=== {name} histogram over {n} resolved frames ===");
    eprintln!(
        "min={:.3}  p50={:.3}  p90={:.3}  p99={:.3}  max={:.3}",
        samples[0],
        p(0.5),
        p(0.9),
        p(0.99),
        samples[n - 1],
    );
}

fn print_sphere_lod(label: &str, state: &RenderState) {
    let sphere_lod = state.sphere_lod_diagnostics();
    eprintln!(
        "{label}: active={} sample_shift={} sample_stride={} source_atoms={} \
         instance_upper_bound={} cull_upper_bound={} base_sample_shift={} \
         viewport_visible_count={} viewport_full_detail={}",
        sphere_lod.active,
        sphere_lod.sample_shift,
        sphere_lod.sample_stride,
        sphere_lod.source_atom_count,
        sphere_lod.instance_upper_bound,
        sphere_lod.cull_upper_bound,
        sphere_lod.base_sample_shift,
        sphere_lod.viewport_visible_count,
        sphere_lod.viewport_full_detail,
    );
}

fn print_stick_lod(label: &str, state: &RenderState) {
    let stick_lod = state.stick_lod_diagnostics();
    eprintln!(
        "{label}: active={} sample_shift={} sample_stride={} source_bonds={} \
         sampled_bond_upper_bound={} cull_upper_bound={} base_sample_shift={} \
         viewport_visible_count={} viewport_full_detail={}",
        stick_lod.active,
        stick_lod.sample_shift,
        stick_lod.sample_stride,
        stick_lod.source_bond_count,
        stick_lod.sampled_bond_upper_bound,
        stick_lod.cull_upper_bound,
        stick_lod.base_sample_shift,
        stick_lod.viewport_visible_count,
        stick_lod.viewport_full_detail,
    );
}

fn sync_with_timing(
    state: &mut RenderState,
    input: &RenderInput<'_>,
) -> (Duration, RenderSyncTimings) {
    let wall_start = Instant::now();
    let timer_start = Instant::now();
    state.sync_with_timer(input, || timer_start.elapsed().as_secs_f64() * 1000.0);
    (wall_start.elapsed(), state.last_sync_timings())
}

fn print_sync_timing(label: &str, wall: Duration, timings: RenderSyncTimings) {
    let scene = timings.scene_store_fragmentation;
    let compaction = timings.scene_store_compaction;
    eprintln!(
        "{label}: wall_ms={:.3} scene_store_object_sync_ms={:.3} \
         scene_store_flush_ms={:.3} rep_sync_ms={:.3} order_bounds_ms={:.3} \
         compute_dispatch_ms={:.3} marker_upload_bytes={} scene_live_mib={:.2} \
         scene_allocated_mib={:.2} scene_orphaned_mib={:.2} scene_capacity_mib={:.2} \
         scene_compacted={} scene_reclaimed_mib={:.2}",
        wall.as_secs_f64() * 1000.0,
        timings.scene_store_object_sync_ms,
        timings.scene_store_flush_ms,
        timings.rep_sync_ms,
        timings.order_bounds_ms,
        timings.compute_dispatch_ms,
        timings.marker_lut_upload_bytes,
        patinae_render::bytes_to_mib(scene.live_bytes),
        patinae_render::bytes_to_mib(scene.allocated_bytes),
        patinae_render::bytes_to_mib(scene.orphaned_bytes),
        patinae_render::bytes_to_mib(scene.capacity_bytes),
        compaction.ran,
        patinae_render::bytes_to_mib(compaction.reclaimed_bytes),
    );
}

fn print_memory_warnings(state: &mut RenderState) {
    for warning in state.take_memory_warnings() {
        eprintln!("renderer-warning: {warning}");
    }
}

struct SceneStoreChurnInput<'a> {
    mol: &'a ObjectMolecule,
    coord: &'a CoordSet,
    atom_colors: &'a [[f32; 4]],
    atom_markers: &'a [u32],
    has_markers: bool,
    settings: &'a ResolvedSettings,
    visible: RepMask,
}

fn run_scene_store_churn(state: &mut RenderState, input: SceneStoreChurnInput<'_>) {
    if std::env::var("BENCH_SCENE_STORE_CHURN").ok().as_deref() != Some("replace") {
        return;
    }
    let iterations = std::env::var("BENCH_SCENE_STORE_CHURN_ITERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(8)
        .max(1);
    let small_atom_count = if input.mol.atom_count() > 3 {
        (input.mol.atom_count() / 2).max(1)
    } else {
        input.mol.atom_count() + 3
    };
    let mut small_mol = synthetic_water_molecule(small_atom_count);
    force_visible_reps(&mut small_mol, input.visible);
    let small_coord = small_mol.current_coord_set().expect("small coord").clone();
    let named = NamedPalette::new();
    let palette = ThemedPalette::dark();
    let resolver = ColorResolver::new(&named, &palette);
    let small_colors: Vec<[f32; 4]> = small_mol
        .atoms()
        .map(|atom| resolver.resolve_atom(atom))
        .collect();
    let small_lod = SceneLod::from_atom_count(small_mol.atom_count());
    let original_lod = SceneLod::from_atom_count(input.mol.atom_count());

    eprintln!(
        "BENCH_SCENE_STORE_CHURN=replace iterations={} alternate_atoms={}",
        iterations, small_atom_count
    );
    for i in 0..iterations {
        let use_small = i % 2 == 0;
        let (active_mol, active_coord, active_colors, lod) = if use_small {
            (&small_mol, &small_coord, small_colors.as_slice(), small_lod)
        } else {
            (input.mol, input.coord, input.atom_colors, original_lod)
        };
        let object_input = RenderObjectInput {
            object_id: ObjectId(1),
            molecule: active_mol,
            coord_set: active_coord,
            transform: IDENTITY_TRANSFORM,
            visible_reps: input.visible,
            draw_reps: input.visible,
            object_settings: None,
            atom_colors: active_colors,
            atom_rep_colors: &[],
            atom_markers: &[],
            marker_updates: &[],
            has_markers: false,
            lod,
            dirty: DirtyFlags::ALL,
        };
        let render_input = RenderInput {
            objects: std::slice::from_ref(&object_input),
            maps: &[],
            strokes: &[],
            settings: input.settings,
            lod,
        };
        let (wall, timings) = sync_with_timing(state, &render_input);
        print_sync_timing("scene_store_churn.replace", wall, timings);
    }

    let restore_input = RenderObjectInput {
        object_id: ObjectId(1),
        molecule: input.mol,
        coord_set: input.coord,
        transform: IDENTITY_TRANSFORM,
        visible_reps: input.visible,
        draw_reps: input.visible,
        object_settings: None,
        atom_colors: input.atom_colors,
        atom_rep_colors: &[],
        atom_markers: input.atom_markers,
        marker_updates: &[],
        has_markers: input.has_markers,
        lod: original_lod,
        dirty: DirtyFlags::ALL,
    };
    let restore_render_input = RenderInput {
        objects: std::slice::from_ref(&restore_input),
        maps: &[],
        strokes: &[],
        settings: input.settings,
        lod: original_lod,
    };
    let (wall, timings) = sync_with_timing(state, &restore_render_input);
    print_sync_timing("scene_store_churn.restore", wall, timings);
}

fn bench_render_loop(c: &mut Criterion) {
    let (width, height) = bench_dims();
    eprintln!("viewport: {}×{}", width, height);
    let scenario = bench_scenario();
    let static_camera = env_flag("BENCH_STATIC_CAMERA");
    let frame_checksum = env_flag("BENCH_FRAME_CHECKSUM");
    eprintln!("BENCH_SCENARIO={}", scenario.name);
    eprintln!("BENCH_STATIC_CAMERA={}", if static_camera { 1 } else { 0 });
    eprintln!(
        "BENCH_FRAME_CHECKSUM={}",
        if frame_checksum { 1 } else { 0 }
    );
    let (device, queue, memory_policy) = request_device();
    let mut mol = load_bench_molecule();
    // Force every atom into the rep set chosen by the scenario or `BENCH_REPS`.
    // patinae-io's CIF loader defaults
    // atoms to LINES only; without this, big assemblies look fast in the
    // bench but emit zero instances. Accepted tokens: cartoon, sticks,
    // spheres, lines, surface, mesh, ribbon, dots.
    let reps_env = std::env::var("BENCH_REPS").unwrap_or_else(|_| scenario.reps.to_string());
    let force_visible: RepMask = reps_env.split(',').map(|s| s.trim().to_lowercase()).fold(
        RepMask(0),
        |acc, name| match name.as_str() {
            "cartoon" => acc.union(RepMask::CARTOON),
            "ribbon" => acc.union(RepMask::RIBBON),
            "sticks" => acc.union(RepMask::STICKS),
            "spheres" => acc.union(RepMask::SPHERES),
            "lines" => acc.union(RepMask::LINES),
            "surface" => acc.union(RepMask::SURFACE),
            "mesh" => acc.union(RepMask::MESH),
            "dots" => acc.union(RepMask::DOTS),
            _ => acc,
        },
    );
    eprintln!(
        "BENCH_REPS={}  → force_visible=0x{:x}",
        reps_env, force_visible.0
    );
    force_visible_reps(&mut mol, force_visible);
    let coord = mol.current_coord_set().expect("coord").clone();

    // Per-atom colors via the same resolver the live host uses.
    let named = NamedPalette::new();
    let palette = ThemedPalette::dark();
    let resolver = ColorResolver::new(&named, &palette);
    let atom_colors: Vec<[f32; 4]> = mol.atoms().map(|a| resolver.resolve_atom(a)).collect();
    let n_atoms = mol.atom_count();
    let marker_mode =
        std::env::var("BENCH_MARKERS").unwrap_or_else(|_| scenario.markers.to_string());
    let atom_markers: Vec<u32> = match marker_mode.as_str() {
        "selected" => vec![MARKER_SELECTED; n_atoms],
        "hover" => {
            let mut markers = vec![0u32; n_atoms];
            if !markers.is_empty() {
                markers[n_atoms / 2] = MARKER_HOVER;
            }
            markers
        }
        "both" => vec![MARKER_SELECTED | MARKER_HOVER; n_atoms],
        _ => Vec::new(),
    };
    let has_markers = atom_markers.iter().any(|&bits| bits != 0);
    eprintln!("BENCH_MARKERS={marker_mode}");
    let n_bonds = mol.bonds().count();
    eprintln!("Loaded molecule: {} atoms, {} bonds", n_atoms, n_bonds);
    {
        // Diagnostic: count per-rep visibility flags actually set on atoms.
        let mut spheres = 0;
        let mut sticks = 0;
        let mut lines = 0;
        let mut cartoon = 0;
        let mut surface = 0;
        let mut ribbon = 0;
        let mut none_rep = 0;
        for a in mol.atoms() {
            let r = a.repr.visible_reps;
            if r.is_visible(RepMask::SPHERES) {
                spheres += 1;
            }
            if r.is_visible(RepMask::STICKS) {
                sticks += 1;
            }
            if r.is_visible(RepMask::LINES) {
                lines += 1;
            }
            if r.is_visible(RepMask::CARTOON) {
                cartoon += 1;
            }
            if r.is_visible(RepMask::SURFACE) {
                surface += 1;
            }
            if r.is_visible(RepMask::RIBBON) {
                ribbon += 1;
            }
            if r.0 == 0 {
                none_rep += 1;
            }
        }
        eprintln!(
            "atom.repr per-rep counts: spheres={spheres} sticks={sticks} lines={lines} \
             cartoon={cartoon} ribbon={ribbon} surface={surface} none={none_rep}"
        );
    }

    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let mut state = RenderState::with_config(
        device.clone(),
        queue.clone(),
        format,
        (width, height),
        RenderConfig {
            picking: PickingMode::Reprojected,
            memory: memory_policy,
            ..Default::default()
        },
    );
    let fxaa_enabled = env_flag("BENCH_FXAA");
    state.set_fxaa(fxaa_enabled);
    eprintln!("BENCH_FXAA={}", if fxaa_enabled { 1 } else { 0 });
    let shadows_enabled = scenario.shadows || env_flag("BENCH_SHADOWS");
    if shadows_enabled {
        let map_size = std::env::var("BENCH_SHADOW_MAP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(512);
        let bias = std::env::var("BENCH_SHADOW_BIAS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.01);
        let intensity = std::env::var("BENCH_SHADOW_INTENSITY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.5);
        let pcf = std::env::var("BENCH_SHADOW_PCF")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);
        state.set_shadows(true, map_size, bias, intensity, pcf);
        eprintln!(
            "BENCH_SHADOWS=1 map_size={map_size} bias={bias} intensity={intensity} pcf={pcf}"
        );
    } else {
        eprintln!("BENCH_SHADOWS=0");
    }
    let skripkin_enabled = scenario.skripkin || env_flag("BENCH_SKRIPKIN");
    let skripkin_dirs = std::env::var("BENCH_SKRIPKIN_DIRS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);
    let skripkin_map = std::env::var("BENCH_SKRIPKIN_MAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(128);
    let skripkin_bias = std::env::var("BENCH_SKRIPKIN_BIAS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.025);
    let skripkin_intensity = std::env::var("BENCH_SKRIPKIN_INTENSITY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    if skripkin_enabled {
        state.set_skripkin_ao(
            true,
            skripkin_dirs,
            skripkin_map,
            skripkin_bias,
            skripkin_intensity,
        );
        eprintln!(
            "BENCH_SKRIPKIN=1 dirs={skripkin_dirs} map={skripkin_map} bias={skripkin_bias} \
             intensity={skripkin_intensity} forced_rebuild={}",
            scenario.force_atlas_rebuild
        );
    } else {
        eprintln!("BENCH_SKRIPKIN=0");
    }
    let silhouettes_enabled = scenario.silhouettes || env_flag("BENCH_SILHOUETTES");
    let silhouette_width = std::env::var("BENCH_SILHOUETTE_WIDTH")
        .ok()
        .and_then(|s| s.parse().ok())
        // Matches the patinae-settings default silhouette width.
        .unwrap_or(2.0);
    if silhouettes_enabled {
        state.set_silhouette(true, silhouette_width, [0.0, 0.0, 0.0, 1.0]);
        eprintln!("BENCH_SILHOUETTES=1 width={silhouette_width}");
    } else {
        state.set_silhouette(false, silhouette_width, [0.0, 0.0, 0.0, 1.0]);
        eprintln!("BENCH_SILHOUETTES=0");
    }
    print_memory_warnings(&mut state);

    // Fit the molecule's bounding sphere into the orthographic frame.
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for atom_idx in 0..n_atoms {
        let p = coord
            .get_atom_coord(AtomIndex(atom_idx as u32))
            .unwrap_or(Vec3::new(0.0, 0.0, 0.0));
        for k in 0..3 {
            let c = [p.x, p.y, p.z][k];
            if c < lo[k] {
                lo[k] = c;
            }
            if c > hi[k] {
                hi[k] = c;
            }
        }
    }
    let center = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ];
    let half_extent = [
        (hi[0] - lo[0]) * 0.5,
        (hi[1] - lo[1]) * 0.5,
        (hi[2] - lo[2]) * 0.5,
    ];
    let scene_half = half_extent[0]
        .max(half_extent[1])
        .max(half_extent[2])
        .max(1.0);

    state.uniforms.view = rotation_view(0.0, center, scene_half * 3.0);
    state.uniforms.view_inv = rigid_view_inverse(&state.uniforms.view);
    state.uniforms.proj = build_orthographic(scene_half * 1.2);
    state.uniforms.view_proj = mat4_mul(&state.uniforms.proj, &state.uniforms.view);
    state.uniforms.set_scene_max_depth(scene_half * 10.0);
    state.ctx.upload_frame(&state.uniforms);

    let settings = default_settings();

    let target_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bench.target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Warm: do a few full syncs+renders so all buffers exist + scene_store
    // settles, then transition to camera-only frames.
    let mut rotation: f32 = 0.0;
    let visible = force_visible;
    let lod = SceneLod::from_atom_count(n_atoms);
    let input = RenderObjectInput {
        object_id: ObjectId(1),
        molecule: &mol,
        coord_set: &coord,
        transform: IDENTITY_TRANSFORM,
        visible_reps: visible,
        draw_reps: visible,
        object_settings: None,
        atom_colors: &atom_colors,
        atom_rep_colors: &[],
        atom_markers: &atom_markers,
        marker_updates: &[],
        has_markers,
        lod,
        dirty: DirtyFlags::ALL,
    };
    let initial_render_input = RenderInput {
        objects: std::slice::from_ref(&input),
        maps: &[],
        strokes: &[],
        settings: &settings,
        lod,
    };
    let (initial_wall, initial_timings) = sync_with_timing(&mut state, &initial_render_input);
    print_sync_timing("cartoon_toggle.first_sync", initial_wall, initial_timings);
    print_memory_warnings(&mut state);
    print_sphere_lod("sphere_lod_initial", &state);
    print_stick_lod("stick_lod_initial", &state);

    // Warm-up + initial poll so all compute builds finish.
    for _ in 0..3 {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        state.render(&target_view, &mut encoder);
        queue.submit(std::iter::once(encoder.finish()));
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
    }
    print_sphere_lod("sphere_lod_after_warmup", &state);
    print_stick_lod("stick_lod_after_warmup", &state);
    run_scene_store_churn(
        &mut state,
        SceneStoreChurnInput {
            mol: &mol,
            coord: &coord,
            atom_colors: &atom_colors,
            atom_markers: &atom_markers,
            has_markers,
            settings: &settings,
            visible,
        },
    );

    if scenario.name == "cartoon_toggle_large" {
        let mut hidden_draw = visible;
        hidden_draw.set_hidden(RepMask::CARTOON);
        let hide_input = RenderObjectInput {
            object_id: ObjectId(1),
            molecule: &mol,
            coord_set: &coord,
            transform: IDENTITY_TRANSFORM,
            visible_reps: visible,
            draw_reps: hidden_draw,
            object_settings: None,
            atom_colors: &atom_colors,
            atom_rep_colors: &[],
            atom_markers: &atom_markers,
            marker_updates: &[],
            has_markers,
            lod,
            dirty: DirtyFlags::DRAW_MASK,
        };
        let hide_render_input = RenderInput {
            objects: std::slice::from_ref(&hide_input),
            maps: &[],
            strokes: &[],
            settings: &settings,
            lod,
        };
        let (hide_wall, hide_timings) = sync_with_timing(&mut state, &hide_render_input);
        print_sync_timing("cartoon_toggle.hide_draw_mask", hide_wall, hide_timings);

        let show_input = RenderObjectInput {
            object_id: ObjectId(1),
            molecule: &mol,
            coord_set: &coord,
            transform: IDENTITY_TRANSFORM,
            visible_reps: visible,
            draw_reps: visible,
            object_settings: None,
            atom_colors: &atom_colors,
            atom_rep_colors: &[],
            atom_markers: &atom_markers,
            marker_updates: &[],
            has_markers,
            lod,
            dirty: DirtyFlags::DRAW_MASK,
        };
        let show_render_input = RenderInput {
            objects: std::slice::from_ref(&show_input),
            maps: &[],
            strokes: &[],
            settings: &settings,
            lod,
        };
        let (show_wall, show_timings) = sync_with_timing(&mut state, &show_render_input);
        print_sync_timing(
            "cartoon_toggle.second_show_draw_mask",
            show_wall,
            show_timings,
        );
    }

    if frame_checksum {
        let mut checksums = Vec::with_capacity(8);
        for _ in 0..8 {
            let frame_input = RenderObjectInput {
                object_id: ObjectId(1),
                molecule: &mol,
                coord_set: &coord,
                transform: IDENTITY_TRANSFORM,
                visible_reps: visible,
                draw_reps: visible,
                object_settings: None,
                atom_colors: &atom_colors,
                atom_rep_colors: &[],
                atom_markers: &atom_markers,
                marker_updates: &[],
                has_markers,
                lod,
                dirty: DirtyFlags::empty(),
            };
            state.sync(&RenderInput {
                objects: std::slice::from_ref(&frame_input),
                maps: &[],
                strokes: &[],
                settings: &settings,
                lod,
            });
            checksums.push(rendered_frame_checksum(
                &device,
                &queue,
                &mut state,
                &target_texture,
                &target_view,
                width,
                height,
            ));
        }
        checksums.sort_unstable();
        checksums.dedup();
        eprintln!(
            "BENCH_FRAME_CHECKSUM unique={} values={:?}",
            checksums.len(),
            checksums
        );
        while state.take_frame_stats().is_some() {}
    }

    let mut group = c.benchmark_group("render_loop");
    group.warm_up_time(Duration::from_secs(2));
    group.measurement_time(Duration::from_secs(8));
    group.sample_size(30);

    let mut opaque_samples: Vec<f32> = Vec::with_capacity(8192);
    let mut fast_overlay_samples: Vec<f32> = Vec::with_capacity(8192);
    let mut translucent_samples: Vec<f32> = Vec::with_capacity(8192);
    let mut composite_samples: Vec<f32> = Vec::with_capacity(8192);
    let mut shadow_samples: Vec<f32> = Vec::with_capacity(8192);
    let mut atlas_ao_samples: Vec<f32> = Vec::with_capacity(8192);
    let mut marking_samples: Vec<f32> = Vec::with_capacity(8192);
    let mut cull_samples: Vec<f32> = Vec::with_capacity(8192);
    let mut total_samples: Vec<f32> = Vec::with_capacity(8192);
    group.bench_function(scenario.name, |b| {
        b.iter(|| {
            if !static_camera {
                rotation += 0.01;
            }
            if skripkin_enabled && scenario.force_atlas_rebuild {
                let bias = skripkin_bias + (rotation.sin().abs() * 0.000_001);
                state.set_skripkin_ao(true, skripkin_dirs, skripkin_map, bias, skripkin_intensity);
            }
            if !static_camera {
                state.uniforms.view = rotation_view(rotation, center, scene_half * 3.0);
                state.uniforms.view_inv = rigid_view_inverse(&state.uniforms.view);
                state.uniforms.view_proj = mat4_mul(&state.uniforms.proj, &state.uniforms.view);
                state.ctx.upload_frame(&state.uniforms);
            }

            // Camera-only update unless BENCH_STATIC_CAMERA=1; sync still runs
            // but most reps see empty dirty.
            let frame_input = RenderObjectInput {
                object_id: ObjectId(1),
                molecule: &mol,
                coord_set: &coord,
                transform: IDENTITY_TRANSFORM,
                visible_reps: visible,
                draw_reps: visible,
                object_settings: None,
                atom_colors: &atom_colors,
                atom_rep_colors: &[],
                atom_markers: &atom_markers,
                marker_updates: &[],
                has_markers,
                lod,
                dirty: DirtyFlags::empty(),
            };
            state.sync(&RenderInput {
                objects: std::slice::from_ref(&frame_input),
                maps: &[],
                strokes: &[],
                settings: &settings,
                lod,
            });
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            state.render(&target_view, &mut encoder);
            queue.submit(std::iter::once(encoder.finish()));
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            // Drain whatever stats are ready. Append to histogram.
            while let Some(s) = state.take_frame_stats() {
                if s.gpu_opaque_ms >= 0.0 {
                    opaque_samples.push(s.gpu_opaque_ms);
                }
                if s.gpu_fast_overlay_ms >= 0.0 {
                    fast_overlay_samples.push(s.gpu_fast_overlay_ms);
                }
                if s.gpu_translucent_ms >= 0.0 {
                    translucent_samples.push(s.gpu_translucent_ms);
                }
                if s.gpu_composite_ms >= 0.0 {
                    composite_samples.push(s.gpu_composite_ms);
                }
                if s.gpu_shadow_ms >= 0.0 {
                    shadow_samples.push(s.gpu_shadow_ms);
                }
                if s.gpu_atlas_ao_ms >= 0.0 {
                    atlas_ao_samples.push(s.gpu_atlas_ao_ms);
                }
                if s.gpu_marking_ms >= 0.0 {
                    marking_samples.push(s.gpu_marking_ms);
                }
                if s.gpu_cull_ms >= 0.0 {
                    cull_samples.push(s.gpu_cull_ms);
                }
                if s.gpu_total_ms >= 0.0 {
                    total_samples.push(s.gpu_total_ms);
                }
            }
            black_box(());
        });
    });

    group.finish();
    print_sphere_lod("sphere_lod_final", &state);
    print_stick_lod("stick_lod_final", &state);

    print_histogram("gpu_total_ms", &mut total_samples);
    print_histogram("gpu_opaque_ms", &mut opaque_samples);
    print_histogram("gpu_fast_overlay_ms", &mut fast_overlay_samples);
    print_histogram("gpu_translucent_ms", &mut translucent_samples);
    print_histogram("gpu_composite_ms", &mut composite_samples);
    print_histogram("gpu_shadow_ms", &mut shadow_samples);
    print_histogram("gpu_atlas_ao_ms", &mut atlas_ao_samples);
    print_histogram("gpu_marking_ms", &mut marking_samples);
    print_histogram("gpu_cull_ms", &mut cull_samples);
    eprintln!("=== GPU Memory Snapshot ===");
    eprintln!("{}", state.memory_snapshot().timing_line());

    // Final per-pass stats summary. We render one more frame, wait for
    // GPU completion, then drain the FrameStatsCollector.
    let mut last_stats = None;
    for _ in 0..6 {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        state.render(&target_view, &mut encoder);
        queue.submit(std::iter::once(encoder.finish()));
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        if let Some(s) = state.take_frame_stats() {
            last_stats = Some(s);
        }
        // Yield briefly so map_async callback can run.
        std::thread::sleep(Duration::from_millis(10));
        let t0 = Instant::now();
        while last_stats.is_none() && t0.elapsed() < Duration::from_millis(50) {
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                })
                .unwrap();
            if let Some(s) = state.take_frame_stats() {
                last_stats = Some(s);
                break;
            }
        }
    }
    if let Some(s) = last_stats {
        eprintln!("=== Last-frame FrameStats ===");
        eprintln!("cpu_sync_ms       = {:.3}", s.cpu_sync_ms);
        eprintln!("cpu_record_ms     = {:.3}", s.cpu_record_ms);
        eprintln!("cpu_total_ms      = {:.3}", s.cpu_total_ms);
        eprintln!("gpu_compute_build = {:.3}", s.gpu_compute_build_ms);
        eprintln!("gpu_cull          = {:.3}", s.gpu_cull_ms);
        eprintln!("gpu_shadow        = {:.3}", s.gpu_shadow_ms);
        eprintln!("gpu_atlas_ao      = {:.3}", s.gpu_atlas_ao_ms);
        eprintln!("atlas_ao_rebuilt  = {}", s.atlas_ao_rebuilt);
        eprintln!("atlas_ao_reused   = {}", s.atlas_ao_reused);
        eprintln!("atlas_ao_dirs     = {}", s.atlas_ao_directions);
        eprintln!("atlas_ao_tile     = {}", s.atlas_ao_tile_size);
        eprintln!("gpu_picking       = {:.3}", s.gpu_picking_ms);
        eprintln!("gpu_picking_repj  = {:.3}", s.gpu_picking_reproject_ms);
        eprintln!("gpu_opaque        = {:.3}", s.gpu_opaque_ms);
        eprintln!("gpu_fast_overlay  = {:.3}", s.gpu_fast_overlay_ms);
        eprintln!("gpu_translucent   = {:.3}", s.gpu_translucent_ms);
        eprintln!("gpu_composite     = {:.3}", s.gpu_composite_ms);
        eprintln!("gpu_marking       = {:.3}", s.gpu_marking_ms);
        eprintln!("gpu_silhouette    = {:.3}", s.gpu_silhouette_ms);
        eprintln!("gpu_ssao          = {:.3}", s.gpu_ssao_ms);
        eprintln!("gpu_ssao_blur     = {:.3}", s.gpu_ssao_blur_ms);
        eprintln!("gpu_ssao_compose  = {:.3}", s.gpu_ssao_compose_ms);
        eprintln!("gpu_fxaa          = {:.3}", s.gpu_fxaa_ms);
        eprintln!("gpu_total_ms      = {:.3}", s.gpu_total_ms);
    } else {
        eprintln!("(no FrameStats available — adapter probably lacks TIMESTAMP_QUERY)");
    }
}

criterion_group!(benches, bench_render_loop);
criterion_main!(benches);
