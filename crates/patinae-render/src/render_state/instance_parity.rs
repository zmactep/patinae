//! GPU parity of compact source copies and independently materialized objects.

use std::sync::Arc;

use lin_alg::f32::Vec3;
use patinae_mol::instancing::{materialize_molecule, InstanceGroup, InstanceTable, ObjectInstance};
use patinae_mol::{
    Atom, AtomFlags, AtomIndex, AtomResidue, BondOrder, DirtyFlags, Element, MoleculeBuilder,
    ObjectMolecule, RepMask,
};
use patinae_settings::{ResolvedSettings, Settings};

use crate::capture::capture_rgba;
use crate::{
    ObjectId, PickingMode, RenderAtomColors, RenderConfig, RenderInput, RenderMemoryPolicy,
    RenderObjectInput, RenderState, SceneLod, IDENTITY_TRANSFORM,
};

fn source(rep: RepMask) -> ObjectMolecule {
    let mut builder = MoleculeBuilder::new("copy-parity");
    for chain in ["A", "B"] {
        for residue in 0..4 {
            let residue_data =
                Arc::new(AtomResidue::from_parts(chain, "ALA", residue + 1, ' ', ""));
            for (name, delta, element) in [
                ("CA", [0.0, 0.0, 0.0], Element::Carbon),
                ("C", [0.7, 0.4, 0.0], Element::Carbon),
                ("O", [0.0, 0.6, 0.8], Element::Oxygen),
            ] {
                let mut atom = Atom::new(name, element);
                atom.residue = Arc::clone(&residue_data);
                atom.state.flags = AtomFlags::PROTEIN;
                atom.repr.visible_reps = rep;
                atom.anisou = Some([0.4, 0.8, 1.2, 0.0, 0.0, 0.0]);
                // The full source is far outside every tested camera frustum.
                let position = Vec3::new(
                    1000.0 + residue as f32 * 0.4 + delta[0],
                    (residue as f32 - 1.5) * 2.0 + delta[1] + if chain == "B" { 10.0 } else { 0.0 },
                    delta[2],
                );
                builder = builder.add_atom(atom, position);
            }
        }
    }
    let mut molecule = builder.build();
    for start in (0..24).step_by(3) {
        molecule
            .add_bond(AtomIndex(start), AtomIndex(start + 1), BondOrder::Single)
            .unwrap();
        molecule
            .add_bond(
                AtomIndex(start + 1),
                AtomIndex(start + 2),
                BondOrder::Single,
            )
            .unwrap();
    }
    molecule
}

fn object<'a>(
    molecule: &'a ObjectMolecule,
    colors: &'a [[f32; 4]],
    rep: RepMask,
    id: u32,
    instances: Option<&'a InstanceTable>,
) -> RenderObjectInput<'a> {
    RenderObjectInput {
        object_id: ObjectId(id),
        instances,
        molecule,
        coord_set: molecule.get_coord_set(0).unwrap(),
        transform: IDENTITY_TRANSFORM,
        visible_reps: rep,
        draw_reps: rep,
        object_settings: None,
        colors: RenderAtomColors::Separate {
            base: colors,
            reps: &[],
        },
        atom_markers: &[],
        recent_atom_markers: None,
        marker_updates: &[],
        has_markers: false,
        lod: SceneLod::Auto,
        dirty: DirtyFlags::ALL,
    }
}

#[test]
#[ignore = "requires a real GPU; verifies copy-specific recent marker pixels"]
fn recent_atom_markers_render_only_on_picked_copies() {
    use crate::RecentAtomMarker;

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: crate::required_limits_for_memory_policy(
            &adapter.limits(),
            RenderMemoryPolicy::performance(),
        ),
        ..Default::default()
    }))
    .unwrap();
    let device = Arc::new(device);
    let queue = Arc::new(queue);
    let molecule = MoleculeBuilder::new("markers")
        .add_atom(Atom::new("CA", Element::Carbon), Vec3::new(0.0, 0.0, 0.0))
        .build();
    let table = InstanceTable {
        groups: vec![InstanceGroup { indices: vec![] }],
        copies: [-4.0, 4.0]
            .into_iter()
            .map(|x| {
                let mut transform = IDENTITY_TRANSFORM;
                transform[3][0] = x;
                ObjectInstance {
                    group: 0,
                    transform,
                }
            })
            .collect(),
    };
    let colors = [[0.4, 0.4, 0.4, 1.0]];
    let settings = ResolvedSettings::resolve(&Settings::default(), None);
    let marker = |copy| RecentAtomMarker {
        atom_index: 0,
        instance: Some(copy),
    };
    for memory in [
        RenderMemoryPolicy::performance(),
        RenderMemoryPolicy::lite(),
    ] {
        let mut renderer = RenderState::with_config(
            Arc::clone(&device),
            Arc::clone(&queue),
            wgpu::TextureFormat::Rgba8Unorm,
            (256, 256),
            RenderConfig {
                memory,
                ..Default::default()
            },
        );
        let mut frame = renderer.uniforms;
        frame.view[3][2] = -100.0;
        frame.view_inv[3][2] = 100.0;
        frame.proj = [
            [0.1, 0.0, 0.0, 0.0],
            [0.0, 0.1, 0.0, 0.0],
            [0.0, 0.0, -0.001, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        frame.view_proj = frame.proj;
        frame.view_proj[3][2] = 0.1;
        frame.proj_inv = [
            [10.0, 0.0, 0.0, 0.0],
            [0.0, 10.0, 0.0, 0.0],
            [0.0, 0.0, -1000.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        for x_shift in [0.0, 1.0] {
            let mut capture = |recent: &[RecentAtomMarker], dirty| {
                let mut input_object =
                    object(&molecule, &colors, RepMask::SPHERES, 1, Some(&table));
                input_object.transform[3][0] = x_shift;
                input_object.atom_markers = &[crate::scene_store::marker::MARKER_RECENT];
                input_object.recent_atom_markers = Some(recent);
                input_object.dirty = dirty;
                capture_rgba(
                    &mut renderer,
                    256,
                    256,
                    &frame,
                    &RenderInput {
                        objects: &[input_object],
                        maps: &[],
                        strokes: &[],
                        settings: &settings,
                        lod: SceneLod::Auto,
                    },
                )
                .unwrap()
            };
            let baseline = capture(&[], DirtyFlags::ALL);
            for (recent, expected) in [
                (vec![marker(0)], [true, false]),
                (vec![marker(1)], [false, true]),
                (vec![marker(0), marker(1)], [true, true]),
                (vec![marker(1)], [false, true]),
                (vec![], [false, false]),
            ] {
                // Copy switches deliberately leave the source bit and dirty flags unchanged.
                let pixels = capture(&recent, DirtyFlags::empty());
                let mut changed = [0usize; 2];
                for (index, (actual, base)) in pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(baseline.as_chunks::<4>().0)
                    .enumerate()
                {
                    if actual != base {
                        changed[usize::from(index % 256 >= 128)] += 1;
                    }
                }
                for side in 0..2 {
                    if expected[side] {
                        assert!(
                            changed[side] > 20,
                            "missing marker: {memory:?}/{x_shift}/{recent:?}: {changed:?}"
                        );
                    } else {
                        assert_eq!(
                            changed[side], 0,
                            "marker leaked to another copy: {memory:?}/{x_shift}/{recent:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
#[ignore = "requires a real GPU; explicit compact/materialized parity acceptance"]
fn compact_copies_match_materialized_all_representations() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("GPU required");
    eprintln!("Instance parity adapter: {:?}", adapter.get_info());
    let memory = RenderMemoryPolicy::performance();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: crate::required_limits_for_memory_policy(&adapter.limits(), memory),
        ..Default::default()
    }))
    .unwrap();
    let device = Arc::new(device);
    let queue = Arc::new(queue);
    let make_state = || {
        RenderState::with_config(
            Arc::clone(&device),
            Arc::clone(&queue),
            wgpu::TextureFormat::Rgba8Unorm,
            (256, 256),
            RenderConfig {
                picking: PickingMode::FullRecord,
                memory,
                ..Default::default()
            },
        )
    };
    let mut compact = make_state();
    let mut explicit = make_state();
    let mut settings = ResolvedSettings::resolve(&Settings::default(), None);
    settings.surface.individual_chains = true;
    // Uniform colors isolate geometry and lighting from ownership seams.
    let colors = vec![[0.3, 0.7, 0.9, 1.0]; 24];
    for rep in [
        RepMask::SPHERES,
        RepMask::STICKS,
        RepMask::LINES,
        RepMask::DOTS,
        RepMask::ELLIPSOIDS,
        RepMask::CARTOON,
        RepMask::RIBBON,
        RepMask::SURFACE,
        RepMask::MESH,
    ] {
        let source = source(rep);
        for count in [1usize, 10, 100] {
            let side = (count as f32).sqrt().ceil() as usize;
            let table = InstanceTable {
                groups: vec![InstanceGroup {
                    indices: (0..12).collect(),
                }],
                copies: (0..count)
                    .map(|copy| {
                        // Exact quarter turn tests both positions and ellipsoid axes.
                        let mut transform = [
                            [0.0, 1.0, 0.0, 0.0],
                            [-1.0, 0.0, 0.0, 0.0],
                            [0.0, 0.0, 1.0, 0.0],
                            [0.0, -1000.0, 0.0, 1.0],
                        ];
                        transform[3][0] += (copy % side) as f32 * 12.0 - (side - 1) as f32 * 6.0;
                        transform[3][1] += (copy / side) as f32 * 12.0 - (side - 1) as f32 * 6.0;
                        ObjectInstance {
                            group: 0,
                            transform,
                        }
                    })
                    .collect(),
            };
            let materialized: Vec<_> = table
                .copies
                .iter()
                .map(|copy| {
                    materialize_molecule(
                        &source,
                        0,
                        &InstanceTable {
                            groups: table.groups.clone(),
                            copies: vec![copy.clone()],
                        },
                    )
                    .unwrap()
                })
                .collect();
            let compact_objects = [object(&source, &colors, rep, 1, Some(&table))];
            let explicit_objects: Vec<_> = materialized
                .iter()
                .enumerate()
                .map(|(index, molecule)| {
                    object(molecule, &colors[..12], rep, index as u32 + 1, None)
                })
                .collect();
            let input = |objects| RenderInput {
                objects,
                maps: &[],
                strokes: &[],
                settings: &settings,
                lod: SceneLod::Auto,
            };
            let half = side as f32 * 6.0 + 5.0;
            let mut frame = compact.uniforms;
            frame.view[3][2] = -100.0;
            frame.view_inv[3][2] = 100.0;
            frame.proj = [
                [1.0 / half, 0.0, 0.0, 0.0],
                [0.0, 1.0 / half, 0.0, 0.0],
                [0.0, 0.0, -0.001, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ];
            frame.view_proj = frame.proj;
            frame.view_proj[3][2] = 0.1;
            frame.proj_inv = [
                [half, 0.0, 0.0, 0.0],
                [0.0, half, 0.0, 0.0],
                [0.0, 0.0, -1000.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ];
            let compact_pixels =
                capture_rgba(&mut compact, 256, 256, &frame, &input(&compact_objects)).unwrap();
            let explicit_pixels =
                capture_rgba(&mut explicit, 256, 256, &frame, &input(&explicit_objects)).unwrap();
            assert!(
                compact_pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel != &compact_pixels[..4]),
                "empty compact {rep:?}/{count}"
            );
            let error: u64 = compact_pixels
                .iter()
                .zip(&explicit_pixels)
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum();
            let mean_error = error as f64 / compact_pixels.len() as f64;
            eprintln!("compact parity {rep:?}, copies={count}, mean channel error={mean_error:.5}");
            let source_buffers = |state: &RenderState| {
                [
                    state.scene.scene_store.atoms.buffer().unwrap().size(),
                    state.scene.scene_store.coords.buffer().unwrap().size(),
                    state.scene.scene_store.color_lut.buffer().unwrap().size(),
                    state.scene.scene_store.bonds.buffer().unwrap().size(),
                ]
            };
            let compact_stats = compact.scene.scene_store.memory_stats();
            let explicit_stats = explicit.scene.scene_store.memory_stats();
            let compact_rep_bytes: u64 = compact
                .scene
                .reps
                .values()
                .map(|entry| entry.rep.memory_usage().capacity_bytes)
                .sum();
            eprintln!("GPU bytes {rep:?}/{count}: compact atom/coord/color/bond={:?}, explicit={:?}; store live/allocated/capacity compact={}/{}/{}, explicit={}/{}/{}; compact rep capacity={compact_rep_bytes}",
    source_buffers(&compact), source_buffers(&explicit), compact_stats.live_bytes, compact_stats.allocated_bytes, compact_stats.capacity_bytes,
    explicit_stats.live_bytes, explicit_stats.allocated_bytes, explicit_stats.capacity_bytes);

            if rep == RepMask::MESH {
                for (name, pixels) in [
                    ("compact", &compact_pixels),
                    ("materialized", &explicit_pixels),
                ] {
                    image::RgbaImage::from_raw(256, 256, pixels.clone())
                        .unwrap()
                        .save(format!(
                            "/tmp/patinae-instance-{}-{count}-{name}.png",
                            rep.0
                        ))
                        .unwrap();
                }
            }
            let compact_bounds = pixel_bounds(&compact_pixels);
            let materialized_bounds = pixel_bounds(&explicit_pixels);
            eprintln!("pixel bounds {rep:?}/{count}: compact={compact_bounds:?}, materialized={materialized_bounds:?}");
            for (compact_axis, explicit_axis) in compact_bounds.into_iter().zip(materialized_bounds)
            {
                assert!(
                    compact_axis.abs_diff(explicit_axis) <= 2,
                    "materialization changes projected extent: {rep:?}/{count}"
                );
            }
            if rep != RepMask::MESH {
                assert!(
                    mean_error < 1.0,
                    "compact/materialized image mismatch: {rep:?}/{count}: {mean_error}"
                );
            }
            // A materialized surface is tessellated again on a world-aligned
            // voxel grid. Wireframe triangle diagonals therefore need not be
            // invariant under rotation. Isolate renderer transform correctness
            // with explicit objects retaining exactly the source geometry.
            let subset_source = materialize_molecule(
                &source,
                0,
                &InstanceTable {
                    groups: table.groups.clone(),
                    copies: vec![ObjectInstance {
                        group: 0,
                        transform: IDENTITY_TRANSFORM,
                    }],
                },
            )
            .unwrap();
            let controls: Vec<_> = table
                .copies
                .iter()
                .enumerate()
                .map(|(index, copy)| {
                    let mut result =
                        object(&subset_source, &colors[..12], rep, index as u32 + 1, None);
                    result.transform = copy.transform;
                    result
                })
                .collect();
            let control_pixels =
                capture_rgba(&mut explicit, 256, 256, &frame, &input(&controls)).unwrap();
            let control_error: u64 = compact_pixels
                .iter()
                .zip(&control_pixels)
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum();
            eprintln!("exact geometry transform control {rep:?}/{count}: channel error sum={control_error}");
            if rep == RepMask::MESH {
                let background = &compact_pixels[..4];
                let changed_pixels = compact_pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(control_pixels.as_chunks::<4>().0.iter())
                    .filter(|(left, right)| left != right)
                    .count();
                let changed_coverage = compact_pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(control_pixels.as_chunks::<4>().0.iter())
                    .filter(|(left, right)| (*left == background) != (*right == background))
                    .count();
                let occupied = compact_pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|pixel| *pixel != background)
                    .count();
                image::RgbaImage::from_raw(256, 256, control_pixels.clone())
                    .unwrap()
                    .save(format!("/tmp/patinae-instance-256-{count}-control.png"))
                    .unwrap();
                eprintln!("mesh transform coverage {count}: changed_pixels={changed_pixels}, changed_coverage={changed_coverage}, occupied={occupied}");
                // Independent compute appends can reorder overlapping wireframe
                // edges. Require identical geometric coverage, permitting only
                // the line-intersection shading ambiguity already present in
                // the explicit mesh renderer.
                assert_eq!(changed_coverage, 0, "same-geometry mesh changes coverage");
                let occupied_mean_error = control_error as f64 / (occupied.max(1) * 4) as f64;
                assert!(occupied_mean_error < 1.0, "mesh transform shading changes by at least one channel code per occupied pixel");
                assert_eq!(pixel_bounds(&control_pixels), compact_bounds);
            } else {
                assert_eq!(
                    control_error, 0,
                    "same-geometry explicit transform differs: {rep:?}/{count}"
                );
            }

            assert_eq!(
                compact.scene.scene_store.atoms.cpu().len(),
                source.atom_count()
            );
            assert_eq!(
                compact.scene.scene_store.coords.cpu().len(),
                source.atom_count()
            );
            assert_eq!(
                compact.scene.scene_store.color_lut.cpu().len(),
                source.atom_count()
            );
            if rep == RepMask::SPHERES {
                let mut hits = 0;
                for y in (8..248).step_by(8) {
                    for x in (8..248).step_by(8) {
                        if let Some(hit) = compact.pick(x, y) {
                            let other = explicit
                                .pick(x, y)
                                .expect("materialized copy at same pixel");
                            assert_eq!(hit.object_id, ObjectId(1));
                            assert_eq!(hit.instance, Some(other.object_id.0 - 1));
                            assert_eq!(hit.atom_id, other.atom_id);
                            hits += 1;
                        }
                    }
                }
                assert!(hits > 0, "visible offscreen-source copy must be pickable");
            }
        }
    }
}

fn pixel_bounds(pixels: &[u8]) -> [u32; 4] {
    let mut bounds = [256, 256, 0, 0];
    for (index, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
        if pixel == &pixels[..4] {
            continue;
        }
        let x = index as u32 % 256;
        let y = index as u32 / 256;
        bounds[0] = bounds[0].min(x);
        bounds[1] = bounds[1].min(y);
        bounds[2] = bounds[2].max(x);
        bounds[3] = bounds[3].max(y);
    }
    bounds
}
