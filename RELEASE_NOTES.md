# Patinae v0.5.1

Last time, Patinae started looking suspiciously like an operating system. This time, it learns to open its window before getting everything else ready.

0.5.1 focuses on a quicker start, fewer rendering surprises, and plugins you can load and unload without restarting the application. The window appears before plugin discovery and graphics preparation (mostly shaders compilation), visual settings update when you change them, and a new Plugins dialog lets you choose what stays loaded.

---

## Highlights

### The window comes first

Patinae now draws its first frame before discovering plugins and preparing the viewport's graphics pipelines. Graphics preparation runs in the background, and plugins that support background registration can initialize there too. Built-in commands remain available while preparation continues; plugin commands and panels appear as their libraries finish loading.

Startup scripts still wait for plugins and graphics to be ready, so they can use the commands they need. Individual plugin failures are reported without stopping the remaining load attempts. Older plugins that require registration on the main thread remain supported, though a slow registration callback can still briefly stall the window.

The change brings the application window onto the screen sooner. It does not make plugin initialization or graphics preparation disappear; it moves that work out of the way of the first frame.

### Load and unload plugins from the sidebar

Open **Plugins** in the sidebar, even with an empty scene. The dialog shows loaded libraries and saved entries that could not be loaded, with descriptions and file paths. Use **Add** to select a library, remove entries you no longer need, and press **Save** to persist and apply the list.

Removed plugins are unloaded from the host; additions go through the loader while unchanged plugins keep running. Commands, settings, handlers, and panels follow their owning plugin, and removing a plugin restores registrations it had replaced. Tasks whose executor has been removed receive a failure instead of remaining active indefinitely. A failed addition is reported individually.

The saved list lives in `plugins.toml`. The first manifest found in the discovery order defines the complete set of libraries to load, with relative or absolute paths. Without a manifest, Patinae keeps the existing directory-scanning behavior. An invalid manifest reports an error rather than silently loading everything in the directory.

**On Linux, unloading removes the plugin from Patinae but keeps its library mapped for safe thread cleanup.** Reloading does not reset static state; restart the application to use a rebuilt plugin binary. This also fixes a crash that could occur when a thread exited after unloading a native plugin.

For plugin authors, this release retains **plugin ABI 7** and **runtime wire version 21**. Background registration is an explicit capability; build the application and bundled plugins together when adopting it. The [plugin authoring guide](https://github.com/zmactep/patinae/blob/v0.5.1/docs/make-your-own-plugin.md#background-registration-and-native-startup) covers the startup contract and Linux behavior.

---

## Bug fixes

- **Cartoon transparency updates immediately.** Changing transparency now moves cached cartoon geometry into the correct rendering pass, instead of leaving the previous appearance on screen.
- **Map display modes switch cleanly.** Switching between `isosurface` and `isomesh` updates the rendering mode before uploading replacement geometry, fixing an assertion failure in debug builds.
- **Solvent-excluded surfaces face outward.** Surface triangles now agree with their outward normals, correcting face orientation for rendering and back-face culling.

Regression coverage now checks transparency across molecular representations and map modes, along with GPU surface geometry and rendering behavior.

## Other improvements

- **A proper macOS disk image.** The DMG now opens with a branded Retina background, large icons, and a compact Finder layout for dragging Patinae into Applications.
- **Stronger regression checks.** Tests now cover more numerical results, serialization round trips, task ownership, native plugin loading, and rendering behavior. CI includes binding and dynamic ABI checks, with core tests kept independent of concrete plugin runtimes.

---

## Downloads

### Bundles

Packages with the application, packaged plugins, and runtime pieces.

| Platform | Architecture | Download |
| --- | --- | --- |
| macOS | Apple Silicon | [Patinae.dmg](https://github.com/zmactep/patinae/releases/download/v0.5.1/Patinae.dmg) |
| Windows | x86_64 | [Windows bundle](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-bundle-windows-x86_64.zip) |
| Windows | ARM64 | [Windows ARM64 bundle](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-bundle-windows-arm64.zip) |

### Standalone executables

| Platform | Architecture | Executable | Plugins |
| --- | --- | --- | --- |
| macOS | Apple Silicon | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-macos-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-plugins-macos-arm64.tar.gz) |
| Windows | x86_64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-windows-x86_64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-plugins-windows-x86_64.zip) |
| Windows | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-windows-arm64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-plugins-windows-arm64.zip) |
| Linux | x86_64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-linux-x86_64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-plugins-linux-x86_64.tar.gz) |
| Linux | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-linux-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-plugins-linux-arm64.tar.gz) |

The AI plugin currently requires a separate source build; the release workflow does not yet include it in plugin archives. The local `make plugins` target does include it. Follow the [AI plugin guide](https://github.com/zmactep/patinae/blob/v0.5.1/plugins/ai/README.md) to install the library and create its configuration.

### Python and web

| Package | Download |
| --- | --- |
| Python wheels | [PyPI package](https://pypi.org/project/patinae/) or release assets |
| Web viewer (WASM + JS) | [patinae-web.tar.gz](https://github.com/zmactep/patinae/releases/download/v0.5.1/patinae-web.tar.gz) |

---

**Full Changelog:** [v0.5.0 → v0.5.1](https://github.com/zmactep/patinae/compare/v0.5.0...v0.5.1)
