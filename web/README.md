# Patinae Web Viewer

Embeddable molecular visualization using WebAssembly and WebGPU. The browser
viewer shares Patinae's Rust scene, renderer and command engine, with a
JavaScript API, optional panels and a `<patinae-viewer>` custom element.

The web crate builds independently of the native workspace. It does not load
native dynamic-library plugins or the embedded Python runtime. For Python in
notebooks, see the [Python package](../python/README.md).

## Run locally

Requirements:

- Rust and Cargo, with the `wasm32-unknown-unknown` target.
- Node.js and npm; the repository's CI uses Node.js 22.
- A WebGPU-capable browser and GPU. WebGPU requires a secure context: use HTTPS
  in deployment and localhost for development. See [WebGPU browser support](https://developer.mozilla.org/en-US/docs/Web/API/WebGPU_API#browser_compatibility).

From the repository root:

```bash
cd web
rustup target add wasm32-unknown-unknown
npm ci
npm run build
npm run dev
```

Open `/examples/index.html` at the URL printed by Vite.

| Example | Purpose |
| --- | --- |
| [index.html](examples/index.html) | Viewer with controls and panels |
| [minimal.html](examples/minimal.html) | Minimal programmatic embedding |
| [perf.html](examples/perf.html) | Performance inspection using TypeScript sources |

The first two examples import `dist/patinae-viewer.js`, so build before opening
them and rebuild after changing TypeScript. `npm run dev` rebuilds WASM and
starts Vite; it does not rebuild the `dist` JavaScript bundle. The performance
example imports TypeScript directly through the development server.

## Embed in a page

Build with `npm run build` and copy the **whole `dist/` directory** to a location
served by your website, such as `/patinae/`. Keep the JavaScript chunks and
`patinae_web_bg.wasm` together. Then add:

```html
<div id="viewer" style="width: 100%; height: 600px"></div>

<script type="module">
  import { PatinaeViewer } from "/patinae/patinae-viewer.js";

  const viewer = new PatinaeViewer(document.getElementById("viewer"), {
    picking: true,
  });
  await viewer.init();

  const reply = await viewer.loadUrl("https://models.rcsb.org/1IGT.bcif.gz", {
    name: "1IGT",
    format: "bcif",
  });
  if ("Err" in reply.result) throw new Error(reply.result.Err);

  for (const id of reply.task_ids) {
    const task = await viewer.tasks.wait(id, 30_000);
    if (task.state !== "succeeded") {
      throw new Error(`Loading ${id}: ${JSON.stringify(task.outcome)}`);
    }
  }

  await viewer.execute("show cartoon");
  await viewer.execute("color green, chain A");

  // Call viewer.destroy() when your application removes this viewer.
</script>
```

Give the container an explicit height. `init()` must complete before loading
data or executing commands. Remote structure URLs must allow browser requests
from your origin through CORS.

For declarative embedding, register the custom element instead:

```html
<script type="module">
  import { registerElement } from "/patinae/patinae-viewer.js";
  registerElement();
</script>

<patinae-viewer
  style="display: block; width: 100%; height: 600px"
  src="https://models.rcsb.org/1IGT.bcif.gz"
  command="show cartoon">
</patinae-viewer>
```

The element waits for loading before running `command` and releases the viewer
when removed. Use the JavaScript API for explicit task and error handling.

## Commands, loading and tasks

`execute()` and `loadUrl()` return a command receipt, not a promise that all
background work has finished:

```json
{"result":{"Ok":null},"messages":[],"task_ids":[]}
```

Check `result`, retain `task_ids` and wait before issuing commands that depend
on loaded data. A failed command can still contain accepted task IDs; inspect
them instead of repeating the whole command. `loadData(bytes, name, format)`
parses a `Uint8Array` synchronously and returns the same receipt shape without
creating a background task.

| API | Behavior |
| --- | --- |
| `viewer.tasks.get(id)` | Read the retained snapshot and outcome |
| `viewer.tasks.list({ active_only: true })` | Read task summaries; use `next_cursor` as `after` for another page |
| `viewer.tasks.wait(id, timeoutMs)` | Wait for a terminal snapshot; timeout is in milliseconds and does not cancel work |
| `viewer.tasks.cancel(id)` | Request cancellation; inspect the task for acknowledgement |

Terminal states are `succeeded`, `failed` and `cancelled`. A completed wait can
return any of them, so always check the state. Cancellation does not roll back
scene changes. Results live in bounded, in-memory history and may expire.
`tasks.changed` events signal that a fresh snapshot is available; the Rust
runner remains the source of task state.

## Interaction and panels

Pass options to the `PatinaeViewer` constructor:

| Option | Use |
| --- | --- |
| `picking: true` | Enable atom picking at initialization; listen for `atom-picked` |
| `selectionOverlay` | Control selection and hover visuals; defaults to the picking choice |
| `defer: true` | Keep the viewer hidden until `await viewer.show()` |
| `memoryProfile` | Choose `auto`, `performance`, `balanced`, `lite` or `manual:<MiB>` |
| `layout` and `slots` | Place `repl`, `objects`, `sequence` and `movie` panels in supplied DOM containers |

Panel containers and page layout belong to the embedding application. The
simple `panels` option expects an element with `id="sidebar"`. Panel CSS is
available in [viewer.css](ts/src/styles/viewer.css); import it into your app or
copy it into your static assets, since the library does not bundle it.

Use `viewer.on(event, callback)` and `viewer.off(event, callback)` for events
such as `ready`, `atom-picked`, `command-output`, `objects-changed` and
`tasks.changed`. Queries include `getObjectNames()`, `getObjectInfo(name)`,
`getSequenceData()` and `countAtoms(selection)`. See the [public API](ts/src/core/api.ts)
and [option/result types](ts/src/core/types.ts) for complete signatures.

## Build and validate

Run these commands from `web/`:

```bash
npm run build
npm exec -- tsc --noEmit
cargo fmt -- --check
cargo check --locked --target wasm32-unknown-unknown
cargo clippy --locked --target wasm32-unknown-unknown --all-targets -- -D warnings
```

`npm run wasm:build` rebuilds only the WASM package in `pkg/`. `npm run build`
also creates the ES module library in `dist/`. The npm-installed `wasm-pack`
binary is used by both scripts; a separate global installation is unnecessary.

This is a library build: `dist/` contains assets, not a standalone HTML site.
`npm run preview` serves that directory, so it does not provide the example
pages. Use the development server for the supplied examples and your own HTML
page when deploying the library.

Serve `.wasm` as `application/wasm`, preserve relative asset paths, and deploy
files from the same build together. A WASM 404 usually means a missing asset
or incorrect base path. An initialization failure can also indicate missing
WebGPU support or insufficient GPU limits; inspect the browser console.

## Source layout

| Path | Responsibility |
| --- | --- |
| `src/` | Rust/WASM host, GPU initialization, input and task integration |
| `ts/src/core/` | Public facade, viewer lifecycle, events and types |
| `ts/src/panels/` | Optional REPL, object, sequence and movie panels |
| `examples/` | Pages for development and embedding examples |
| `pkg/`, `dist/` | Generated WASM bindings and browser library assets |

Shared domain behavior belongs in the appropriate [core crate](../crates/).
Browser adapters handle browser I/O and scheduling; TypeScript consumes the
Rust command and task contracts. Native plugin development is covered in
[Make your own Patinae plugin](../docs/make-your-own-plugin.md).

Licensed under [BSD-3-Clause](LICENSE).
