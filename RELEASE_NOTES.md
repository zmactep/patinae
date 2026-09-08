# Patinae v0.4.7

A virus capsid asks a surprisingly practical question of a molecular viewer: how many times do you need to store the same protein to show the whole shell?

Performance work in 0.4.7 started with the less photogenic side of that question: copies of molecular data made while saving a session, atom records occupying GPU memory, and colors kept in intermediate buffers. Session saving now streams from borrowed molecular data, GPU atom records are smaller, and colors go directly into renderer storage. Each change removes something that large structures used to make us pay for.

Then the same idea becomes visible. A capsid contains repeated subunits, arranged by symmetry. Store their source atoms once, keep the transformations that place each copy, and use that shared geometry to draw the assembly. That is the foundation of `biounit`, the headline feature of Patinae 0.4.7.

There is more to this release than larger assemblies. **AI agents can ask the running application which plugins, formats, and settings are available.** Recent Atoms now feeds directly into command-line selections and measurements. File handling and GPU compatibility also receive fixes that matter before a structure ever reaches the screen.

---

## Highlights

### From a structure file to the biological assembly

`biounit` uses biological assembly definitions retained from PDB, mmCIF, and BinaryCIF files. It creates a new object from one source coordinate state, placing copies according to the assembly operators supplied by the file.

```text
fetch 1LP3, name=source, type=cif, async=0
biounit source, name=capsid, assembly=1
hide everything, capsid
show cartoon, capsid
color cyan, capsid
disable source
zoom capsid
```

Omit `assembly` to use the first definition in the file, or add `state=2` to choose another source state. The new assembly is an independent snapshot: you can delete the source object and keep working with the assembly.

Copies share source atoms, colors, and representations. The renderer reuses their geometry with individual transformations, including in shadow, transparency, and picking passes. Object information distinguishes stored atoms from displayed atoms, making the effect of repetition visible.

### Repeated subunits, exact atom picks

Picking remembers which copy you clicked. Recent Atoms markers stay on that copy, and labels and measurements use its displayed coordinates. The new one-based `instance` selector lets selections address particular copies.

Whole assemblies can be translated, rotated, and aligned while retaining compact storage. `translate` also gains `center=1`, which places an object's bounding-box center at a specified destination—the operation used to arrange the capsids around the circle.

When you need independent edits, expand the assembly in place:

```text
materialize capsid
```

The object keeps its name, appearance, and placement. Measurements, recent picks, and saved selection membership are remapped to the expanded atoms. Each copy then has explicit coordinates and can be edited independently.

### Less memory spent between operations

The memory improvements also apply to ordinary molecular objects:

- **Session saving** streams from borrowed molecular snapshots, avoiding a full owned snapshot just to serialize the scene.
- **Session loading** restores shared residue and name storage within each molecule.
- **GPU atom records** are reduced to 16 bytes, with corresponding updates to renderer and raytracer data layouts.
- **Color updates** resolve directly into renderer staging storage, removing persistent intermediate color copies. Captures and exports reuse the staged colors.

Compact assemblies build on those changes by sharing source atom, coordinate, color, and bond buffers across copies. Drawing more copies still costs rendering work; the storage model avoids duplicating their full molecular data.

### Runtime discovery for AI agents

An AI agent preparing a molecular scene needs to know what the particular Patinae process can do. A plugin may be installed but fail to load; a format may support reading but not writing; a setting may only exist after a plugin registers it. `capabilities` makes that information available through the same command interface the agent uses to operate the viewer.

| Command | What the agent learns |
| --- | --- |
| `capabilities` | Available discovery topics: plugins, formats, and settings. |
| `capabilities plugins` | Successfully loaded plugins, with their declared names, versions, and descriptions. |
| `capabilities formats` | The four format categories that can be inspected. |
| `capabilities formats run` | Script suffixes accepted by `run` in this process. |
| `capabilities formats load` | Structure and other file suffixes accepted by `load`. |
| `capabilities formats load_traj` | Trajectory suffixes accepted by `load_traj`. |
| `capabilities formats save` | Output suffixes accepted by `save`. |
| `capabilities settings` | An alphabetically sorted, deduplicated list of built-in and currently registered dynamic setting names. |

For example, before choosing a script or output format, an agent can query:

```text
capabilities plugins
capabilities formats run
capabilities formats save
capabilities settings
```

It can then choose from the reported formats and setting names, and verify the relevant plugin's version. The same queries help people diagnose why a script works in one installation but fails in another.

The answers reflect the active command handlers and build features. A reader-only plugin format appears under `load`; a writer-only format appears under `save`. Built-in handler precedence is respected, and an unavailable command category is reported as `(unavailable)`. The plugin list contains instances that completed loading and registration, rather than a scan of library files on disk.

Output is deterministic plain text with fixed headings and explicit empty results, suitable for both a REPL transcript and agent tooling. Discovery does not require a loaded molecule or a renderer. Help, completion, and argument validation cover the new topics. `capabilities settings` is a name index; it does not claim to return setting values or a complete parameter schema.

### Recent Atoms reaches the command line

The Recent Atoms list is now available as `pk1` through `pkN`, with `pk*` selecting their union. Numbering follows the current list order.

After picking the required atoms, a bare command creates the corresponding measurement:

```text
distance
angle
dihedral
```

These use the first two, three, or four recent atoms respectively. Explicit selection arguments remain available for scripted measurements.

The aliases are live: removing a recent atom renumbers later entries, clearing the list clears their targets, and requesting a missing numbered alias produces an error. `pk*` is the union of the current recent atoms. They can be used in ordinary selection expressions, connecting a user's viewport picks to commands issued by a script or an AI agent.

`dist` also supports the bare distance shorthand. Each bare measurement command allocates a new object using the first free name for its kind, such as `distance01` or `angle02`, without overwriting existing objects. Too few picks or undefined geometry produce an error without leaving an empty measurement object behind.

---

## New features

- **Biological assemblies:** `biounit` constructs compact assemblies from file-provided definitions.
- **Explicit expansion:** `materialize` converts a compact object into independently editable atoms in place.
- **Copy-aware interaction:** instance selections, exact copy picking, anchored labels, and measurements use displayed positions.
- **Compact object movement:** whole-object transforms and alignment preserve shared storage; `translate center=1` supports precise scene layout.
- **PRS v4:** sessions preserve instance tables and copy-specific annotation anchors. Legacy raw, v2, and v3 sessions remain readable, and `prs-upgrade` is updated.
- **Recent Atoms aliases:** `pk1`–`pkN` and `pk*` connect picking to selection expressions and bare measurement commands.
- **Runtime discovery for AI agents:** `capabilities plugins` and the four `capabilities formats` leaves report the effective capabilities of the running process in deterministic plain text.
- **Settings discovery:** `capabilities settings` exposes built-in and active plugin setting names through the same interface, with help and completion.

## Bug fixes and improvements

### PDB chain boundaries survive repeated identifiers

Some PDB files reuse a one-character chain identifier for separate segments divided by `TER`. Those segments previously collapsed into one internal chain. Patinae now keeps them distinct as `A`, `A2`, `A3`, and so on, with tracking reset at model boundaries. This makes the segments separately addressable when inspecting or selecting chains.

PDB output maps these internal identifiers back to their original one-character source identifiers and preserves segment boundaries and secondary-structure records. The change includes handling for blank chain identifiers, subsets, multiple models, and round trips.

### Compressed saves finish before reporting success

Saving a compressed molecular file could previously report success before the gzip encoder had finished, leaving an incomplete output stream. Patinae now finishes compression and the underlying buffered write before returning success, and reports errors that occur during finalization.

Save dispatch and capability reporting also agree on built-in compound gzip suffixes, including mixed-case input. PRS, PML, and plugin-owned formats retain their own dispatch rules. This connects the file-writing fix to runtime discovery: the output formats advertised to an agent follow the same suffix rules used by `save`.

### Two GPU compatibility fixes

The lighting uniform buffer previously exceeded the 16 KiB binding limit used by downlevel GPU configurations. Its directional capacity is now sized to fit that limit, and the Rust and shader declarations are kept consistent. Regression checks cover both the buffer size and the matching shader capacity. ([#28](https://github.com/zmactep/patinae/pull/28))

Device creation now explicitly requests compute-capable limits, fixing the Windows configuration where inherited limits could not support the workgroups and storage buffers required by the renderer. A regression check verifies the requested compute dimensions, invocation capacity, and storage-buffer support. ([#29](https://github.com/zmactep/patinae/pull/29))

Thanks to **David Hyunyoo Jang** (@daylight-00) for both GPU compatibility fixes.

### Memory, validation, and build maintenance

The two memory commits address separate costs: session snapshots and atom storage first, then persistent color copies between the scene and renderer. Their regression coverage includes serialization, mutation after loading shared data, shader and raytracer layouts, and rendered-image and picking parity. The color-storage change also preserves reuse for captures and geometry exports.

Rust 1.98 Clippy fixes replace manual slice filling and fixed-size chunk handling with the corresponding slice operations while preserving remainder behavior. Local agent, Compound Engineering, and planning artifacts are excluded from version control. Release metadata is synchronized across the workspace, Python, web packages, lockfiles, Makefile, and README badge.

### Working with compact assemblies

Colors and representations are shared across copies. Independent copy styling, partial structural edits, atom-file and geometry exports, and plugin ray tracing require `materialize` first. PRS saving and viewport image capture support compact objects directly.

Spatial predicates such as `within` and `around` currently operate in source coordinates within each copy; they are not full cross-copy contact searches. Selection and hover highlights may repeat across copies, while Recent Atoms markers identify the exact picked copy. Compact surfaces describe source subsets separately; after materialization, recalculated surfaces can reflect contacts between copies and produce different tessellation.

---

## Downloads

### Bundles

Self-contained packages with the app, plugins, and runtime pieces.

| Platform | Architecture | Download |
| --- | --- | --- |
| macOS | Apple Silicon | [Patinae.dmg](https://github.com/zmactep/patinae/releases/download/v0.4.7/Patinae.dmg) |
| Windows | x86_64 | [patinae-bundle-windows-x86_64.zip](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-bundle-windows-x86_64.zip) |
| Windows | ARM64 | [patinae-bundle-windows-arm64.zip](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-bundle-windows-arm64.zip) |

### Standalone executables

| Platform | Architecture | Executable | Plugins |
| --- | --- | --- | --- |
| macOS | Apple Silicon | [Executable](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-macos-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-plugins-macos-arm64.tar.gz) |
| Windows | x86_64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-windows-x86_64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-plugins-windows-x86_64.zip) |
| Windows | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-windows-arm64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-plugins-windows-arm64.zip) |
| Linux | x86_64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-linux-x86_64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-plugins-linux-x86_64.tar.gz) |
| Linux | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-linux-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-plugins-linux-arm64.tar.gz) |

### Python and web

| Package | Download |
| --- | --- |
| Python wheels | [PyPI package](https://pypi.org/project/patinae/) or release assets |
| Web viewer (WASM + JS) | [patinae-web.tar.gz](https://github.com/zmactep/patinae/releases/download/v0.4.7/patinae-web.tar.gz) |

---

## Release scale

The feature and fix work since v0.4.6 spans **12 commits** and **151 files**, with **10,573 insertions** and **1,595 deletions**, before the version bump and these notes.

---

**Full Changelog:** [v0.4.6 → v0.4.7](https://github.com/zmactep/patinae/compare/v0.4.6...v0.4.7)
