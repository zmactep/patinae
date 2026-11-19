# Website viewer bundle

The generated `patinae-viewer.js`, `patinae_web-q1teE-kv.js`, and
`patinae_web_bg.wasm` were built from Patinae commit
`4eaac6c5a2931dcebca632f4b73c0387e46c7257` (package version 0.5.3)
with `npm run build` in `web/`, using the committed Cargo and npm lockfiles.

This build supports browser downloads of sessions, selected coordinates, and
PNG images at custom resolutions. It includes the browser startup timing fix
from `b078ac0` and the stick rendering fix from `ea7a2a6`.
The published v0.5.3 web archive predates these
fixes and fails at viewer initialization with `time not implemented on this
platform`; do not replace this bundle with that archive.

The shared GPU layouts also include the storage-buffer compatibility fix from
`d2736dd`: unused adjacency bindings were removed, reducing the required limit
from ten to eight buffers per shader stage. This restores startup on browsers
that expose only nine buffers. The homepage and PRS tutorial were checked with
an advertised adapter limit of nine and an actual device limit of eight.

Replace all three generated files together when updating the viewer, including
the hashed JavaScript filename. The homepage download links track the latest
stable desktop release independently of this web build.

Site integrations use `viewer-commands.js` to wait for background task completion
before issuing commands that depend on the loaded structure. The current viewer
API uses `execute()` and `tasks.wait()`; `loadUrl()` returns task IDs before the
structure is ready.
