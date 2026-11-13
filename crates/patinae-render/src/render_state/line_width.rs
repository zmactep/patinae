//! GPU coverage for the `line_width` setting: line quads must rasterise at
//! the requested pixel width (scene-wide and per-object) in both projections,
//! stay pickable across the widened footprint, survive frustum culling near
//! the screen edge, and clip cleanly at the near plane.

use std::sync::Arc;

use patinae_mol::{
    Atom, AtomIndex, BondOrder, CoordSet, DirtyFlags, Element, ObjectMolecule, RepMask,
};
use patinae_settings::{ResolvedSettings, Settings};

use super::stick_precision::frame;
use crate::capture::capture_rgba;
use crate::{
    ObjectId, PickingMode, RenderAtomColors, RenderConfig, RenderInput, RenderMemoryPolicy,
    RenderObjectInput, RenderState, SceneLod, IDENTITY_TRANSFORM,
};

const SIZE: u32 = 256;
// The bond runs along x at y = 0, i.e. across the middle row boundary.
const CENTER_ROW: u32 = SIZE / 2;
// A column inside the first (atom 0) half-bond.
const PROBE_COLUMN: u32 = SIZE / 2 - 40;
// `frame()` frames ±3 Å around the origin at the focal depth.
const HALF_EXTENT: f32 = 3.0;

fn gpu_state() -> RenderState {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&Default::default())).expect("GPU required");
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
    state
}

fn two_atom_lines() -> ObjectMolecule {
    let mut molecule = ObjectMolecule::new("line-width");
    for name in ["C1", "C2"] {
        let mut atom = Atom::new(name, Element::Carbon);
        atom.repr.visible_reps = RepMask::LINES;
        molecule.add_atom(atom);
    }
    molecule
        .add_bond(AtomIndex(0), AtomIndex(1), BondOrder::Single)
        .unwrap();
    molecule
}

/// Renders one two-atom line object; returns RGBA pixels (alpha 0 = empty).
fn render_lines(
    state: &mut RenderState,
    molecule: &ObjectMolecule,
    coords: [f32; 6],
    scene_width: f32,
    object_width: Option<f32>,
    perspective: bool,
) -> Vec<u8> {
    let coords = CoordSet::from_coords(coords.to_vec());
    let colors = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]];
    let mut scene = Settings::default();
    scene.line.width = scene_width;
    let settings = ResolvedSettings::resolve(&scene, None);
    let object_settings = object_width.map(|width| {
        let mut object = scene.clone();
        object.line.width = width;
        ResolvedSettings::resolve(&object, None)
    });
    let object = RenderObjectInput {
        object_id: ObjectId(1),
        molecule,
        coord_set: &coords,
        instances: None,
        transform: IDENTITY_TRANSFORM,
        visible_reps: RepMask::LINES,
        draw_reps: RepMask::LINES,
        object_settings,
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
    capture_rgba(state, SIZE, SIZE, &frame(50.0, perspective), &input).unwrap()
}

fn lit(pixels: &[u8], x: u32, y: u32) -> bool {
    pixels[((y * SIZE + x) * 4 + 3) as usize] > 0
}

fn lit_rows(pixels: &[u8], x: u32) -> u32 {
    (0..SIZE).filter(|&y| lit(pixels, x, y)).count() as u32
}

#[test]
#[ignore = "requires a real GPU; checks line_width rasterisation and picking"]
fn gpu_line_width_controls_rendered_and_picked_width() {
    let mut state = gpu_state();
    let molecule = two_atom_lines();
    let coords = [-2.0, 0.0, 0.0, 2.0, 0.0, 0.0];

    for perspective in [false, true] {
        for (scene_width, object_width) in [
            (1.0, None),
            (1.49, None),
            (4.0, None),
            (9.0, None),
            (1.0, Some(9.0)),
        ] {
            let pixels = render_lines(
                &mut state,
                &molecule,
                coords,
                scene_width,
                object_width,
                perspective,
            );
            let width: f32 = object_width.unwrap_or(scene_width);
            let rows = lit_rows(&pixels, PROBE_COLUMN);
            let min_rows = width.floor() as u32;
            let max_rows = width.ceil() as u32 + 1;
            assert!(
                (min_rows..=max_rows).contains(&rows),
                "perspective={perspective} width={width}: {rows} lit rows, \
                 expected {min_rows}..={max_rows}"
            );

            let center = state
                .pick(PROBE_COLUMN, CENTER_ROW)
                .expect("line centre must be pickable");
            assert_eq!(center.object_id, ObjectId(1));
            assert_eq!(center.atom_id, 0);
            if width >= 9.0 {
                let edge = state
                    .pick(PROBE_COLUMN, CENTER_ROW + 3)
                    .expect("wide line must be pickable off-axis");
                assert_eq!(edge.atom_id, 0);
            }
        }
    }
}

#[test]
#[ignore = "requires a real GPU; checks line culling pad and near-plane clipping"]
fn gpu_wide_lines_survive_edge_cull_and_near_plane_clip() {
    let mut state = gpu_state();
    let molecule = two_atom_lines();
    let px_per_angstrom = SIZE as f32 / (2.0 * HALF_EXTENT);

    // A bond pointing away from the top edge, its nearer atom 6 px above the
    // viewport. The 20 px line's round cap still reaches ~4 px into the image,
    // so frustum culling must not drop it (a fixed 0.1 Å pad would).
    let gap = 6.0 / px_per_angstrom;
    let y0 = HALF_EXTENT + gap;
    let pixels = render_lines(
        &mut state,
        &molecule,
        [0.0, y0, 0.0, 0.0, y0 + 1.5, 0.0],
        20.0,
        None,
        false,
    );
    let top_rows = (0..8).filter(|&y| lit(&pixels, SIZE / 2, y)).count();
    assert!(
        top_rows >= 2,
        "cap of an off-screen wide line was culled: {top_rows} lit rows at the top edge"
    );

    // A bond crossing the near plane and passing behind the eye (camera at
    // z = 50, near = 1). The visible part must render without blowing up.
    let pixels = render_lines(
        &mut state,
        &molecule,
        [-1.0, 0.5, 0.0, -1.0, 0.5, 60.0],
        6.0,
        None,
        true,
    );
    let lit_pixels = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 0)
        .count();
    assert!(lit_pixels > 0, "near-plane-crossing line vanished");
    assert!(
        lit_pixels < (SIZE * SIZE / 4) as usize,
        "near-plane-crossing line covered {lit_pixels} pixels"
    );
}
