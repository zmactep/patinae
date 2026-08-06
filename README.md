<p align="center">
  <img src="images/patinae.png" alt="Patinae" width="200">
</p>

<h1 align="center">Patinae</h1>

<p align="center">
  <strong>Familiar power. Modern core. No legacy baggage.</strong><br>
  A fast, programmable molecular viewer for research, scripting, and the web.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-BSD--3--Clause-blue.svg" alt="License">
  <img src="https://img.shields.io/badge/version-0.4.6-green.svg" alt="Version 0.4.6">
  <img src="https://img.shields.io/badge/GPU-WebGPU-purple.svg" alt="WebGPU">
</p>

---

## What is Patinae?

Patinae is a modern molecular visualization toolkit for interactive research,
scripting, structural analysis, publication images, notebooks, and embedding.

It is built around a responsive GPU-first viewer, a command-driven workflow, and
extension points for native, Python, and web applications. The aim is to keep the
power of a full molecular workstation while making the everyday experience fast,
light, and comfortable.

## Formerly PyMOL-RS

Patinae was called PyMOL-RS through the 0.3.x releases. The project started as
a GPU-first remake experiment: keep the familiar molecular command workflow,
but rebuild the viewer as a modern, independent application.

By 0.4.0, the project had outgrown the remake label. The renderer, command
runtime, plugin host, desktop UI, Python surface, packaging, and web viewer were
redesigned into an independent application and toolkit. The rename to Patinae
marks that shift.

Patinae is not an official PyMOL project or a wrapper around PyMOL source code.
It remains compatible with familiar command and session workflows where that is
useful for users, but the implementation is its own codebase and rendering
stack.

## Highlights

| Area | Design |
| --- | --- |
| **Rendering** | WebGPU via `wgpu`, GPU impostors, compute-heavy pipelines, native and web targets |
| **Workflow** | Interactive command line, object panel, sequence viewer, picking, measurements, sessions |
| **Extensibility** | Rust plugin SDK, Python plugin, Python package, reusable crates |
| **Architecture** | Independent `patinae-*` crates with explicit renderer, scene, command, IO, session, and plugin boundaries |
| **Portability** | Native desktop app, Python/Jupyter package, and WebAssembly/WebGPU viewer |

## Quick Start

### Pre-built binaries

Download the latest release for your platform from
[Releases](https://github.com/zmactep/patinae/releases/latest).

Standalone executable:

```bash
patinae protein.pdb
```

Python package:

```bash
pip install patinae
```

### Build from source

Prerequisites:

- [Rust](https://rustup.rs/) for the core workspace and desktop app.
- [uv](https://docs.astral.sh/uv/) for Python package builds.
- [Node.js](https://nodejs.org/) for the web viewer and notebook widget assets.

```bash
git clone https://github.com/zmactep/patinae
cd patinae
make patinae
./target/release/patinae
```

| Target | Command | Requires |
| --- | --- | --- |
| Patinae release build | `make patinae` | cargo |
| Patinae dev run | `make patinae-dev` | cargo |
| Reference plugins | `make plugins` | cargo |
| Python wheel | `make python-release` | cargo, uv |
| Web viewer | `make web-build` | cargo, npm |
| Rust check | `make check` | cargo |
| Tests | `make test` | cargo |

### Development verification

Run the baseline checks locally before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace
cargo test --workspace
cargo check --manifest-path python/Cargo.toml
rustup target add wasm32-unknown-unknown
cargo check --manifest-path web/Cargo.toml --target wasm32-unknown-unknown
```

Additional compliance tools are useful when installed locally:
`cargo audit` or `cargo deny` for dependency policy, `cargo hack` for feature
matrices, `cargo udeps` for unused dependencies, and Miri for targeted
unsafe-code tests.

## Molecular Capabilities

**Formats:** PDB, mmCIF, bCIF, MOL2, SDF/MOL, XYZ, GRO, CCP4/MRC, trajectory
formats such as XTC/TRR, and gzip-compressed inputs.

**Representations:** Spheres, sticks, lines, cartoon, ribbon, surface
(SAS/SES/VdW), mesh, dots, labels, isomesh, isosurface, and isodot.

**Selections:** atom, residue, chain, object, proximity, polymer, solvent, and
stored-selection expressions with a familiar molecular selection syntax.

```text
chain A and name CA
byres around 5 ligand
polymer and not solvent
```

**Commands:** interactive verbs with completion and scriptability, including
`load`, `fetch`, `show`, `hide`, `color`, `select`, `zoom`, `center`, `orient`,
`png`, `ray`, `align`, `cealign`, `symexp`, `isomesh`, `isosurface`, and
`isodot`.

**Structural analysis:**

- Kabsch superposition and CE structural alignment.
- First-class distance, angle, dihedral, and atom-label collections with shared
  native/web rendering.
- Crystallographic symmetry expansion with all 230 space groups.
- Secondary-structure assignment from geometry.
- Electron density map loading and contouring.

**Sessions:** save and load PRS v3 `.prs` sessions and import legacy `.pse`
sessions for interoperability. The loader remains compatible with legacy raw
sessions and PRS v2. Upgrade PyMOL-RS v0.3.3 or Patinae v0.4.0–v0.4.2 sessions
with `cargo run -p prs-upgrade -- old.prs upgraded.prs`.

**Ray tracing:** the `raytracer` plugin provides offline GPU ray tracing with
BVH acceleration, shadows, transparency, and edge detection.

## Native Desktop App

The native Patinae app is a desktop interface around the same
renderer, scene model, command runtime, and plugin host used by the rest of the
workspace. It includes a command line, object panel, sequence viewer, plugin
panels, mouse picking, native file workflows, and a GPU viewport.

<p align="center">
  <img src="images/interface.png" alt="Patinae interface" width="800">
</p>

### Renderer Memory Profiles

Patinae chooses a renderer memory profile when the GPU device and viewport
renderer are created. This is intentionally a startup-time decision: memory
profiles are not runtime settings, and changing one requires recreating the
renderer, normally by restarting the app, web viewer, or benchmark process.

The desktop app accepts `PATINAE_RENDER_MEMORY_PROFILE`:

```bash
PATINAE_RENDER_MEMORY_PROFILE=balanced patinae protein.pdb
PATINAE_RENDER_MEMORY_PROFILE=lite patinae protein.pdb
PATINAE_RENDER_MEMORY_PROFILE=manual:1024 patinae protein.pdb
```

`render_loop` benchmarks accept the same values through
`BENCH_MEMORY_PROFILE`.

Supported profile values are:

- `performance`: default native behavior with the full interactive rendering
  feature set.
- `balanced`: lower scratch-target pressure while preserving the normal
  interactive feature set.
- `lite`: disables or gates optional allocation-heavy viewport features such as
  SSAO, FXAA scratch targets, selection overlays, and larger shadow or atlas
  resources.
- `manual:<MiB>`: explicit memory budget, for example `manual:1024`.

When no override is provided, Patinae selects a profile from the adapter type,
backend, platform, and GPU limits. Requested WGPU limits are still clamped to
the adapter capabilities before device creation.

## Python And Notebooks

The `patinae` Python package exposes a familiar command object for scripting,
automation, and notebook workflows.

```python
from patinae import cmd

cmd.load("protein.pdb")
cmd.show("cartoon")
cmd.color("green", "chain A")
cmd.select("site", "byres around 5 ligand")
site_atoms = cmd.count_atoms("site")
cmd.do("set ambient, 0.8")
```

The package also includes an anywidget-based Jupyter viewer:

```python
from patinae.widget import Viewer

view = Viewer()
view.show()

cmd = view.get_cmd()
cmd.fetch("1CRN")
cmd.show("cartoon")
cmd.color("green", "chain A")
```

## Web Viewer

Patinae runs in the browser through WebAssembly and WebGPU. The web viewer is
published as `@patinae/viewer` and can be embedded into applications or static
pages.

```html
<div id="viewer" style="width: 800px; height: 600px"></div>

<script type="module">
  import { PatinaeViewer } from "@patinae/viewer";

  const viewer = new PatinaeViewer(document.getElementById("viewer"));
  await viewer.init();

  await viewer.loadUrl("https://models.rcsb.org/1IGT.bcif.gz", {
    name: "1IGT",
    format: "bcif",
  });

  viewer.execute("show cartoon");
  viewer.execute("color green, chain A");
</script>
```

The package can also register a custom element:

```html
<script type="module">
  import { registerElement } from "@patinae/viewer";
  registerElement();
</script>

<patinae-viewer
  src="https://models.rcsb.org/1IGT.bcif.gz"
  command="show cartoon; color green, chain A">
</patinae-viewer>
```

## Architecture

```text
patinae/
├── patinae                    Native desktop application
├── crates/patinae-mol         Molecular data model: atoms, bonds, molecules, coordinate sets
├── crates/patinae-io          Structure, map, session, fetch, and trajectory IO
├── crates/patinae-select      Selection parser and evaluator
├── crates/patinae-color       Named colors, palettes, schemes, and ramps
├── crates/patinae-settings    Configuration and settings system
├── crates/patinae-algos       Alignment, symmetry, sequence, and analysis algorithms
├── crates/patinae-render      WebGPU renderer
├── crates/patinae-scene       Viewer state, scene graph, camera, input, and render bridge
├── crates/patinae-session     `.prs` and `.pse` session support
├── crates/patinae-cmd         Command parser, executor, and command registry
├── crates/patinae-framework   Shared app messaging and component infrastructure
├── crates/patinae-plugin      Plugin SDK
├── crates/patinae-plugin-host Runtime plugin host for front ends
├── plugins                    Reference Rust plugins
├── tools/prs-upgrade          Legacy `.prs` migration utility
├── python                     Python package and Jupyter widget
└── web                        WebAssembly/WebGPU viewer package
```

Each crate is independently usable. Need only the selection parser? Use
`patinae-select`. Need molecular file parsing in a Rust pipeline? Use
`patinae-io` and `patinae-mol`. Need a viewer? Compose the scene, renderer,
command runtime, and front end that fit your application.

## Plugin System

Patinae supports native Rust plugins loaded at startup from
`~/.patinae/plugins/` or bundled beside the application. Plugins can register
commands, provide panels, hook into the command pipeline, and interact with the
viewer through the Patinae plugin APIs.

Plugins are compiled as dynamic libraries (`.dylib` on macOS, `.so` on Linux,
`.dll` on Windows). The workspace includes reference plugins:

| Plugin | Crate | Description |
| --- | --- | --- |
| **hello** | `hello-plugin` | Minimal plugin lifecycle and command-registration example |
| **raytracer** | `raytracer-plugin` | GPU ray tracing with BVH acceleration, shadows, transparency, and edge detection |
| **ipc** | `ipc-plugin` | Inter-process communication plugin for external tool integration |
| **python** | `python-plugin` | Embedded CPython interpreter for scripting inside the native app |

Build and stage the reference plugins:

```bash
make plugins
```

Install them into the user plugin directory:

```bash
make plugins-install
```

### Embedded Python Plugin

<p align="center">
  <img src="images/python-plugin.png" alt="Patinae Python plugin" width="800">
</p>

The `python-plugin` embeds CPython directly into the native application. It is
separate from the standalone Python package: scripts run inside the desktop app
and issue commands through the same dispatcher as native commands.

## License

[BSD 3-Clause](LICENSE)
