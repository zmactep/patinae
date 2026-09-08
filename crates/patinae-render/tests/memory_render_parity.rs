//! GPU regression coverage for scene-buffer layout changes.
//!
//! Run explicitly with `--ignored`. Set PATINAE_GPU_REFERENCE_DIR to compare
//! exact RGBA/picking outputs on the same GPU. PATINAE_GPU_RECORD=1 records
//! the reference before changing the renderer; it is never enabled by default.

use std::path::PathBuf;
use std::sync::Arc;

use patinae_mol::{
    Atom, AtomIndex, BondOrder, CoordSet, DirtyFlags, Element, ObjectMolecule, RepMask,
};
use patinae_render::capture::capture_rgba;
use patinae_render::{
    required_limits_for_memory_policy, ObjectId, PickingMode, RenderConfig, RenderInput,
    RenderMemoryPolicy, RenderObjectInput, RenderState, SceneLod, IDENTITY_TRANSFORM,
};
use patinae_settings::{ResolvedSettings, Settings};

fn reference(name: &str, bytes: &[u8]) {
    let Some(root) = std::env::var_os("PATINAE_GPU_REFERENCE_DIR") else {
        return;
    };
    let path = PathBuf::from(root).join(name);
    if std::env::var("PATINAE_GPU_RECORD").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    } else {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "GPU output changed: {name}"
        );
    }
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
    ] {
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
            }
            let object = RenderObjectInput {
                object_id: ObjectId(3),
                molecule: &mol,
                coord_set: &coords,
                transform: IDENTITY_TRANSFORM,
                visible_reps: rep,
                draw_reps: rep,
                object_settings: None,
                atom_colors: &colors,
                atom_rep_colors: &[],
                atom_markers: &[],
                marker_updates: &[],
                has_markers: false,
                lod: SceneLod::Auto,
                dirty: DirtyFlags::ALL,
            };
            let input = RenderInput {
                objects: &[object],
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
        }
    }
}
