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
  <img src="https://img.shields.io/badge/version-0.5.3-green.svg" alt="Version 0.5.3">
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

Patinae was called PyMOL-RS through the 0.3.x releases and was renamed in 0.4.0
as it grew into an independent application and toolkit. It retains familiar
command and session workflows, with its own codebase and rendering stack.
Patinae is not an official PyMOL project or a wrapper around PyMOL source code.

## Highlights

| Area | Design |
| --- | --- |
| **Rendering** | WebGPU via `wgpu`, GPU impostors, compute-heavy pipelines, native and web targets |
| **Workflow** | Interactive command line, object panel, sequence viewer, picking, measurements, sessions |
| **Extensibility** | Rust plugin SDK, AI and Python plugins, Python package, reusable crates |
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

Install [Rust](https://rustup.rs/) and Make to build the desktop app.
See the [development setup](CONTRIBUTING.md#development-setup) for platform
dependencies and the additional tools used by Python and web builds.

```bash
git clone https://github.com/zmactep/patinae
cd patinae
make patinae
./target/release/patinae
```

For development setup, build targets, and verification, see
[Contributing to Patinae](CONTRIBUTING.md).

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
`png`, `ray`, `align`, `cealign`, `symexp`, `biounit`, `materialize`, `isomesh`, `isosurface`, and
`isodot`.

**Biological assemblies:** `biounit` reads assembly definitions retained from
PDB, mmCIF, or BinaryCIF and creates an independent snapshot of the source's
current coordinate state. An `instanced` object stores source atoms once and
reuses their geometry with copy matrices. Colors and representations apply to
all copies; picking and measurements distinguish individual copies.

```text
biounit protein, name=assembly, assembly=1
color cyan, assembly
materialize assembly
```

The assembly identifier is optional and defaults to the first definition in
the input. `state=2` selects a different source state. `materialize` expands
copies in place, preserving the object name and measurements; the resulting
`explicit` object supports independent atom edits. Ordinary objects default to
`explicit`. PRS sessions and viewport screenshots retain compact assemblies;
atom-file exports, geometry exports, and plugin ray tracing require
`materialize` first. Assembly transforms must be rigid, with at most 65,535
copies per object.

Surfaces in compact form describe the source subsets independently. After
materialization they are recalculated from explicit coordinates; contacts
between copies and wireframe triangulation can therefore change.

**Structural analysis:**

- Kabsch superposition and CE structural alignment.
- First-class distance, angle, dihedral, and atom-label collections with shared
  native/web rendering.
- Crystallographic symmetry expansion with all 230 space groups.
- Secondary-structure assignment from geometry.
- Electron density map loading and contouring.

**Sessions:** save and load PRS v4 `.prs` sessions and import legacy `.pse`
sessions for interoperability. The loader remains compatible with legacy raw
sessions and PRS v2/v3. Upgrade PyMOL-RS v0.3.3 or Patinae v0.4.0–v0.4.2 sessions
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

See [Renderer Memory Profiles](docs/render-memory-profiles.md) for GPU memory
profiles and configuration.

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

Patinae runs in the browser through WebAssembly and WebGPU. The web package
provides a JavaScript API and a `<patinae-viewer>` custom element for embedding
in applications and static pages.

See the [Web Viewer README](web/README.md) for local setup, embedding examples,
loading and task completion, panels, and deployment requirements.

## Plugin System

Patinae supports native Rust plugins loaded at startup from
`~/.patinae/plugins/` or bundled beside the application. Plugins can register
commands, provide panels, hook into the command pipeline, and interact with the
viewer through the Patinae plugin APIs.

Plugins are compiled as dynamic libraries (`.dylib` on macOS, `.so` on Linux,
`.dll` on Windows). The workspace includes reference plugins:

| Plugin | Crate | Description |
| --- | --- | --- |
| **[hello](plugins/hello/README.md)** | `hello-plugin` | Minimal plugin lifecycle and command-registration example |
| **[raytracer](plugins/raytracer/README.md)** | `raytracer-plugin` | GPU ray tracing with BVH acceleration, shadows, transparency, and edge detection |
| **[ipc](plugins/ipc/README.md)** | `ipc-plugin` | Inter-process communication plugin for external tool integration |
| **[python](plugins/python/README.md)** | `python-plugin` | Embedded CPython interpreter for scripting inside the native app |
| **[ai](plugins/ai/README.md)** | `ai-plugin` | Natural-language commands, scene image inspection, and MCP tools through a configurable Responses API endpoint |

See the [plugin overview](plugins/README.md) for installation, discovery, and
verification, and each plugin's README for usage and configuration.

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
