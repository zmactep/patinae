# Raytracer plugin

Render the displayed molecular scene with GPU ray tracing, BVH acceleration,
shadows, transparency, supersampling, and optional outlines. The plugin adds
the `ray` command and a rendering panel with resolution, lighting, and export
controls.

## Build and install

From the repository root, using the same revision as the Patinae application:

```sh
cargo build --locked --release -p raytracer-plugin
mkdir -p ~/.patinae/plugins
cp target/release/libraytracer_plugin.dylib ~/.patinae/plugins/
```

On Linux copy `libraytracer_plugin.so`; on Windows copy `raytracer_plugin.dll`.
Restart Patinae and check `help ray`. See the [plugin overview](../README.md)
for custom directories and building all plugins together.

Ray tracing requires a native host with renderer artifacts and GPU compute
support. The plugin uses the host GPU; it does not provide a CPU fallback.

## Render and save

With a structure loaded and its representations visible, enter:

```pml
ray
ray 1920, 1080, 2
ray width=1920, height=1080, antialias=2, filename=figure.png
```

Without `filename`, the result appears in the viewport. Use `png figure.png`
to save it before interacting with the viewport; interaction dismisses the
rendered image. With `filename`, `ray` exports directly to PNG. A missing or
different file extension is replaced with `.png`.

```text
ray [width [, height [, antialias [, filename [, quiet]]]]]
```

All arguments also accept named form.

| Argument | Meaning | Default |
| --- | --- | --- |
| `width` | Output width in pixels | Viewport width, at least 1024 |
| `height` | Output height in pixels | Viewport height, at least 768 |
| `antialias` | Supersampling level: 1, 2, 3, or 4 | Global `antialias` setting |
| `filename` | PNG output path | Display in the viewport |
| `quiet` | Suppress render feedback with `1` | `0` |

Level 2 renders at twice the width and height before downsampling; level 4
requires sixteen times the output pixel count. Use positive dimensions and
reduce resolution or supersampling when GPU memory is limited.

## Settings

Use Patinae's `set` and `get` commands. Settings take effect on the next render:

```pml
set ray_trace_mode, 1
set ray_shadow, 1
set ray_opaque_background, 0
ray width=1600, height=1200, antialias=2, filename=outlined.png
```

| Setting | Default | Meaning |
| --- | --- | --- |
| `ray_trace_mode` | `0` | 0: normal; 1: normal with outlines; 2: outlines only; 3: quantized colors with outlines |
| `ray_shadow` | `true` | Enable shadows |
| `ray_transparency_shadows` | `true` | Account for transparency in shadows |
| `ray_max_passes` | `25` | Maximum transparency passes, from 1 to 100 |
| `ray_trace_fog` | `-1` | Values above 0 enable fog using scene fog parameters; other values disable it |
| `ray_trace_color` | `-6` | Outline color index; default resolves to black |
| `ray_opaque_background` | `-1` | -1: follow `opaque_background`; 0: transparent; 1: opaque |
| `ray_trace_depth_factor` | `0.1` | Edge gradient direction threshold |
| `ray_trace_slope_factor` | `0.6` | Edge gradient magnitude threshold |
| `ray_trace_disco_factor` | `0.05` | Edge discontinuity threshold |
| `ray_trace_gain` | `0.12` | Edge sampling radius adjustment |

By default, lighting follows the scene's classic shading parameters. Enable
`rt_use_custom` to use the plugin's own lighting values:

```pml
set rt_use_custom, 1
set rt_ambient, 0.2
set rt_specular, 0.4
ray
```

| Custom setting | Default | Range |
| --- | --- | --- |
| `rt_ambient` | `0.14` | 0–1 |
| `rt_direct` | `0.45` | 0–1 |
| `rt_reflect` | `0.45` | 0–1 |
| `rt_specular` | `0.5` | 0–1 |
| `rt_shininess` | `40` | 1–128 |

## Scene limitations

The native command consumes the host's displayed renderer artifacts. Compact
`instanced` objects must be expanded with `materialize object_name` before ray
tracing. Materialization changes the object's storage and can increase memory
use substantially for large assemblies.

If rendering reports unavailable GPU or artifact support, check that the plugin
and host were built together and that the host has a working renderer. An empty
result also warrants checking which objects and representations are visible.

## Source

- [commands.rs](src/commands.rs): command arguments, viewport output, and PNG export.
- [lib.rs](src/lib.rs): plugin registration and setting defaults.
- [panel.rs](src/panel.rs): rendering panel.
- [artifact_gpu](src/artifact_gpu): native renderer artifact processing and GPU work.
- [shaders](src/shaders): ray tracing, edge detection, and compositing.
