//! `RenderState` — public entry point for the crate.
//!
//! It owns the long-lived GPU resources and coordinates scene sync,
//! per-frame compute, draw passes, postprocess, and picking.

mod artifact_flow;
mod compute_build_flow;
mod construction;
mod culling_flow;
mod frame_flow;
mod geometry_export_flow;
mod math;
mod memory_flow;
mod picking_budget;
mod picking_flow;
mod resize;
mod screen_flow;
mod settings;
mod shadow_flow;
pub(crate) mod state;
mod sync_flow;
mod visible_flow;

pub use state::{RenderState, RenderSyncTimings};

#[cfg(test)]
mod memory_render_parity {
    //! GPU regression coverage for scene-buffer layout changes.
    //!
    //! Run with `cargo test -p patinae-render --lib memory_render_parity -- --ignored`.
    //! Set PATINAE_GPU_REFERENCE_DIR to compare
    //! exact RGBA/picking outputs on the same GPU. PATINAE_GPU_RECORD=1 records
    //! the reference before changing the renderer; it is never enabled by default.

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::capture::capture_rgba;
    use crate::{
        required_limits_for_memory_policy, AtomColorSource, ColorLutEntry, ObjectId, PickingMode,
        RenderAtomColors, RenderConfig, RenderInput, RenderMemoryPolicy, RenderObjectInput,
        RenderState, SceneLod, IDENTITY_TRANSFORM,
    };
    use patinae_mol::{
        Atom, AtomIndex, BondOrder, CoordSet, DirtyFlags, Element, ObjectMolecule, RepMask,
    };
    use patinae_settings::{ResolvedSettings, Settings};

    struct TestColors<'a> {
        colors: &'a [[f32; 4]],
        writes: Arc<AtomicUsize>,
    }

    impl AtomColorSource for TestColors<'_> {
        fn len(&self) -> usize {
            self.colors.len()
        }
        fn get(&self, _index: usize) -> Option<ColorLutEntry> {
            panic!("render/export must use staged colors instead of resolving individual atoms")
        }
        fn write(&self, target: &mut [ColorLutEntry]) {
            self.writes.fetch_add(1, Ordering::Relaxed);
            for (target, base) in target.iter_mut().zip(self.colors) {
                *target = ColorLutEntry::new(*base, Default::default());
            }
        }
    }

    // Primitive order can vary because stick caps come from a HashMap and GPU
    // meshes use parallel append buffers. Preserve each triangle's winding/data.
    fn canonical_geometry(geometry: &crate::DisplayedGeometry) -> Vec<String> {
        let mut records = Vec::new();
        for object in &geometry.objects {
            for primitive in &object.primitives {
                if let crate::DisplayedPrimitive::Mesh { rep, mesh } = primitive {
                    for triangle in mesh.vertices.chunks(3) {
                        records.push(format!("{:?} {rep:?} {triangle:?}", object.object_id));
                    }
                } else {
                    records.push(format!("{:?} {primitive:?}", object.object_id));
                }
            }
        }
        records.sort();
        records
    }

    fn reference(name: &str, bytes: &[u8]) {
        let Some(root) = std::env::var_os("PATINAE_GPU_REFERENCE_DIR") else {
            return;
        };
        let path = PathBuf::from(root).join(name);
        if std::env::var("PATINAE_GPU_RECORD").as_deref() == Ok("1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        } else {
            if std::fs::read(&path).unwrap() == bytes {
                return;
            }
            // Overlapping mesh lines have two observed draw-order outcomes on
            // Metal. Accept only exact alternatives recorded from the baseline,
            // never a pixel-error tolerance or a candidate-generated reference.
            if name.starts_with("mesh-") && name.ends_with(".rgba") {
                let prefix = format!("{name}.variant-");
                for entry in std::fs::read_dir(path.parent().unwrap()).unwrap() {
                    let entry = entry.unwrap();
                    if entry.file_name().to_string_lossy().starts_with(&prefix)
                        && std::fs::read(entry.path()).unwrap() == bytes
                    {
                        return;
                    }
                }
            }
            panic!("GPU output changed: {name}");
        }
    }

    fn compare_frame(name: &str, expected: &[u8], actual: &[u8]) {
        if expected == actual {
            return;
        }
        assert!(
            name.starts_with("mesh-") && std::env::var_os("PATINAE_GPU_REFERENCE_DIR").is_some(),
            "rendered pixels changed: {name}"
        );
        reference(name, actual);
    }

    #[test]
    #[ignore = "requires a real GPU; run explicitly for renderer changes"]
    fn scene_layout_preserves_rendering_picking_and_updates() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("GPU adapter required; this test must not silently skip");
        eprintln!("GPU parity adapter: {:?}", adapter.get_info());
        let memory = RenderMemoryPolicy::performance();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: required_limits_for_memory_policy(&adapter.limits(), memory),
            ..Default::default()
        }))
        .unwrap();
        let mut state = RenderState::with_config(
            Arc::new(device),
            Arc::new(queue),
            wgpu::TextureFormat::Rgba8Unorm,
            (128, 128),
            RenderConfig {
                picking: PickingMode::FullRecord,
                memory,
                ..Default::default()
            },
        );
        let mut uniforms = state.uniforms;
        uniforms.view[3][2] = -20.0;
        uniforms.view_inv[3][2] = 20.0;
        uniforms.proj = [
            [0.125, 0.0, 0.0, 0.0],
            [0.0, 0.125, 0.0, 0.0],
            [0.0, 0.0, -0.001, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        uniforms.view_proj = uniforms.proj;
        uniforms.view_proj[3][2] = 0.02;
        uniforms.proj_inv = [
            [8.0, 0.0, 0.0, 0.0],
            [0.0, 8.0, 0.0, 0.0],
            [0.0, 0.0, -1000.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let mut mol = ObjectMolecule::new("layout");
        for (name, element) in [
            ("C", Element::Carbon),
            ("N", Element::Nitrogen),
            ("O", Element::Oxygen),
        ] {
            let mut atom = Atom::new(name, element);
            atom.state.flags = patinae_mol::AtomFlags::PROTEIN;
            atom.anisou = Some([1.0, 2.0, 3.0, 0.0, 0.0, 0.0]);
            mol.add_atom(atom);
        }
        mol.add_bond(AtomIndex(0), AtomIndex(1), BondOrder::Single)
            .unwrap();
        mol.add_bond(AtomIndex(1), AtomIndex(2), BondOrder::Double)
            .unwrap();
        let coords = CoordSet::from_coords(vec![-3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
        let colors = [
            [1.0, 0.1, 0.1, 1.0],
            [0.1, 1.0, 0.1, 1.0],
            [0.1, 0.1, 1.0, 1.0],
        ];
        let settings = ResolvedSettings::resolve(&Settings::default(), None);
        for (name, rep) in [
            ("spheres", RepMask::SPHERES),
            ("sticks", RepMask::STICKS),
            ("lines", RepMask::LINES),
            ("dots", RepMask::DOTS),
            ("ellipsoids", RepMask::ELLIPSOIDS),
            ("surface", RepMask::SURFACE),
            ("mesh", RepMask::MESH),
            ("cartoon", RepMask::CARTOON),
            ("ribbon", RepMask::RIBBON),
        ] {
            let mut mol = mol.clone();
            let mut coords = coords.clone();
            let mut colors = colors.to_vec();
            if rep == RepMask::CARTOON || rep == RepMask::RIBBON {
                let root = std::env::var_os("TEST_STRUCTURES_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_tests")
                    });
                mol = patinae_io::read_file(&root.join("1fsd.cif"))
                    .expect("1fsd.cif is required for backbone parity");
                coords = mol.get_coord_set(0).unwrap().clone();
                colors = (0..mol.atom_count())
                    .map(|i| [0.2 + (i % 3) as f32 * 0.3, 0.5, 0.7, 1.0])
                    .collect();
            }
            for transparent in [false, true] {
                for (i, atom) in mol.atoms_mut().enumerate() {
                    atom.repr.visible_reps = rep;
                    let alpha = if transparent && i == 1 {
                        Some(0.4)
                    } else {
                        None
                    };
                    atom.repr.sphere_transparency = alpha;
                    atom.repr.stick_transparency = alpha;
                    atom.repr.surface_transparency = alpha;
                    atom.repr.ellipsoid_transparency = alpha;
                    atom.repr.cartoon_transparency = alpha;
                }
                let object = RenderObjectInput {
                    object_id: ObjectId(3),
                    molecule: &mol,
                    coord_set: &coords,
                    transform: IDENTITY_TRANSFORM,
                    visible_reps: rep,
                    draw_reps: rep,
                    object_settings: None,
                    colors: crate::RenderAtomColors::Separate {
                        base: &colors,
                        reps: &[],
                    },
                    atom_markers: &[],
                    marker_updates: &[],
                    has_markers: false,
                    lod: SceneLod::Auto,
                    dirty: DirtyFlags::ALL,
                };
                let mut objects = [object];
                let input = RenderInput {
                    objects: &objects,
                    maps: &[],
                    strokes: &[],
                    settings: &settings,
                    lod: SceneLod::Auto,
                };
                let rgba = capture_rgba(&mut state, 128, 128, &uniforms, &input).unwrap();
                assert!(
                    rgba.as_chunks::<4>()
                        .0
                        .iter()
                        .any(|pixel| pixel != &rgba[..4]),
                    "empty {name} frame"
                );
                reference(&format!("{name}-{transparent}.rgba"), &rgba);
                let picks = [40, 64, 88].map(|x| state.pick(x, 64));
                if rep == RepMask::SPHERES {
                    for (i, pick) in picks.iter().enumerate() {
                        let hit = pick.as_ref().expect("sphere center must be pickable");
                        assert_eq!(hit.object_id, ObjectId(3));
                        assert_eq!(hit.atom_id, i as u32);
                    }
                }
                reference(
                    &format!("{name}-{transparent}.picks"),
                    format!("{picks:?}").as_bytes(),
                );
                let options = crate::GeometryExportOptions::default();
                let expected_export = state.export_displayed_geometry(&input, &options).unwrap();
                let writes = Arc::new(AtomicUsize::new(0));
                objects[0].colors = RenderAtomColors::Source(Box::new(TestColors {
                    colors: &colors,
                    writes: Arc::clone(&writes),
                }));
                let input = RenderInput {
                    objects: &objects,
                    maps: &[],
                    strokes: &[],
                    settings: &settings,
                    lod: SceneLod::Auto,
                };
                compare_frame(
                    &format!("{name}-{transparent}.rgba"),
                    &rgba,
                    &capture_rgba(&mut state, 128, 128, &uniforms, &input).unwrap(),
                );
                assert_eq!(
                    format!("{:?}", [40, 64, 88].map(|x| state.pick(x, 64))),
                    format!("{picks:?}")
                );
                assert_eq!(writes.load(Ordering::Relaxed), 1);
                objects[0].dirty = DirtyFlags::empty();
                let input = RenderInput {
                    objects: &objects,
                    maps: &[],
                    strokes: &[],
                    settings: &settings,
                    lod: SceneLod::Auto,
                };
                compare_frame(
                    &format!("{name}-{transparent}.rgba"),
                    &rgba,
                    &capture_rgba(&mut state, 128, 128, &uniforms, &input).unwrap(),
                );
                let actual_export = state.export_displayed_geometry(&input, &options).unwrap();
                assert_eq!(
                    canonical_geometry(&actual_export),
                    canonical_geometry(&expected_export)
                );
                state.render_artifact_snapshot(&input);
                state
                    .for_each_trace_geometry_chunk(&input, &options, &mut |_| Ok(()))
                    .unwrap();
                assert_eq!(
                    writes.load(Ordering::Relaxed),
                    1,
                    "clean frames and exports must reuse staged colors"
                );
            }
        }
    }
}
