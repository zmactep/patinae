# Website viewer bundle

The generated `patinae-viewer.js`, `patinae_web-CEfyJuOx.js`, and
`patinae_web_bg.wasm` were built from Patinae commit
`5c065035f1bbf7a87812885308a8e33ab448c649` (package version 0.5.3)
with `npm run build` in `web/`, using the committed Cargo and npm lockfiles.

This includes the browser startup timing fix from `b078ac0` and the stick
rendering fix from `ea7a2a6`. The published v0.5.3 web archive predates these
fixes and fails at viewer initialization with `time not implemented on this
platform`; do not replace this bundle with that archive.

Replace all three generated files together when updating the viewer, including
the hashed JavaScript filename. The homepage download links track the latest
stable desktop release independently of this web build.

Site integrations use `viewer-commands.js` to wait for background task completion
before issuing commands that depend on the loaded structure. The current viewer
API uses `execute()` and `tasks.wait()`; `loadUrl()` returns task IDs before the
structure is ready.
