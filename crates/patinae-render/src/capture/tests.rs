use super::*;

#[test]
fn converts_premultiplied_rows_to_straight_rgba() {
    let rgba = [100, 20, 10, 128, 3, 2, 1, 255, 30, 20, 10, 0];
    let bgra = [10, 20, 100, 128, 1, 2, 3, 255, 10, 20, 30, 0];
    for (row, swizzle) in [(&rgba, false), (&bgra, true)] {
        let mut pixels = Vec::new();
        append_rgba_row(&mut pixels, row, swizzle, ReadbackAlpha::Straight);
        assert_eq!(pixels, [199, 40, 20, 128, 3, 2, 1, 255, 0, 0, 0, 0]);
        pixels.clear();
        append_rgba_row(&mut pixels, row, swizzle, ReadbackAlpha::Preserve);
        assert_eq!(pixels, rgba);
    }
}
#[test]
#[ignore = "requires a real GPU"]
fn gpu_png_preserves_straight_colors_and_composites_antialiased_edges() {
    use crate::{
        ObjectId, RenderMapInput, RenderMapMode, RenderStrokeInput, SceneLod, StrokeSegment,
        IDENTITY_TRANSFORM,
    };
    use patinae_algos::surface::Grid3D;
    use patinae_settings::{ResolvedSettings, Settings};

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: crate::required_device_limits(&adapter.limits()).unwrap(),
        ..Default::default()
    }))
    .unwrap();
    // An odd width exercises padded rows. BGRA checks the native encoder
    // against the same channel ordering as a common browser surface.
    let size = (97, 81);
    let mut state = RenderState::new(
        device.into(),
        queue.into(),
        wgpu::TextureFormat::Bgra8Unorm,
        size,
    );
    let grid = Grid3D::from_dims(
        [-0.5, -0.5, -0.75],
        [1.0, 1.0, 0.5],
        [1; 3],
        vec![-1.0, -1.0, -1.0, -1.0, 1.0, 1.0, 1.0, 1.0],
    );
    let settings = ResolvedSettings::resolve(&Settings::default(), None);
    let segments = [StrokeSegment::new(
        [-0.8, 0.68, -0.1],
        [0.8, 0.81, -0.1],
        [0.0, 0.0, 0.0, 1.0],
        5.0,
        true,
    )];
    let strokes = [RenderStrokeInput {
        object_id: ObjectId(2),
        segments: &segments,
        bounds: None,
        geometry_revision: 1,
        material_revision: 1,
    }];
    let path = std::env::temp_dir().join(format!(
        "patinae-render-png-test-{}.png",
        std::process::id()
    ));
    let background = [189u8, 111, 223];
    let mut frame = FrameUniforms::default();
    frame.proj[2][2] = -1.0;
    frame.proj_inv = frame.proj;
    frame.view_proj = frame.proj;
    frame.light_intensity = [1.0, 0.0, 0.0, 0.0];
    frame.light_counts[0] = 1.0;

    for fxaa in [false, true] {
        state.set_fxaa(fxaa);
        let mut export = |alpha, opaque| {
            let maps = [RenderMapInput {
                object_id: ObjectId(1),
                grid: &grid,
                mode: RenderMapMode::Isosurface,
                level: 0.0,
                color: [0.13, 0.72, 0.32, alpha],
                transform: IDENTITY_TRANSFORM,
                geometry_revision: 1,
                material_revision: 1,
                dirty: false,
            }];
            let input = RenderInput {
                objects: &[],
                maps: &maps,
                strokes: &strokes,
                settings: &settings,
                lod: SceneLod::Auto,
            };
            state.set_clear_color(background.map(|c| f32::from(c) / 255.0), opaque);
            capture_png(&mut state, &path, size.0, size.1, &frame, &input).unwrap();
            let png = image::open(&path).unwrap().into_rgba8();
            assert_eq!(png.dimensions(), size);
            png
        };
        let solid = export(1.0, false);
        let transparent = export(0.5, false);
        let matte = export(0.5, true);

        let center = transparent.get_pixel(size.0 / 2, size.1 / 2);
        let solid_center = solid.get_pixel(size.0 / 2, size.1 / 2);
        assert!(
            (126..=129).contains(&center[3]),
            "half-opacity surface: {center:?}"
        );
        assert!(
            solid_center[1] > solid_center[0] && solid_center[1] > 20,
            "{solid_center:?}"
        );
        for channel in 0..3 {
            assert!(
                center[channel].abs_diff(solid_center[channel]) <= 2,
                "straight PNG color: {center:?}, solid {solid_center:?}"
            );
        }
        assert_eq!(transparent.get_pixel(0, 0).0, [0; 4]);
        assert!(
            transparent.pixels().any(|p| (1..125).contains(&p[3])),
            "missing partial edge coverage"
        );
        for (foreground, output) in transparent.pixels().zip(matte.pixels()) {
            assert_eq!(output[3], 255);
            let alpha = u32::from(foreground[3]);
            for channel in 0..3 {
                let expected = (u32::from(foreground[channel]) * alpha
                    + u32::from(background[channel]) * (255 - alpha)
                    + 127)
                    / 255;
                assert!(u32::from(output[channel]).abs_diff(expected) <= 2, "fxaa={fxaa}, foreground {foreground:?}, matte {output:?}, channel {channel}, expected {expected}");
            }
        }
    }
    std::fs::remove_file(path).unwrap();
}
