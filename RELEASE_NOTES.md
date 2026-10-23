# Patinae v0.5.2

Patinae 0.5.2 begins a redesign we'll continue over the next few releases. We've also rebuilt the color system with consistent priority rules, updated the graphics stack, and added draggable labels. On macOS, the app is now signed and notarized, so installation no longer requires a trip to the terminal.

## Highlights

### The redesign has started

We've reworked the panels, controls, and menus around a shared visual style. There is more to change, and we'll keep working on it in the next releases, but the interface is already more consistent. Take a look at the tooltips, too.

The theme switch now has three states: **Dark, Light, and System**. Patinae remembers your choice between launches; System follows the OS appearance without restart.

You can now **reset representation colors directly in the Color popover**. Click the cross beside an explicit color to remove that override for the current object or selection. Hovering shows the exact `unset` command.

Double-click an object or selection name to rename it in place. File opening, saving, and plugin selection now use native dialogs on all desktop platforms.

### Keep everything up-to-date

Patinae now uses **Slint 1.18.1 and WGPU 30**. We've also refreshed the web viewer's dependencies, including its Rust/WASM stack, DOMPurify, and Vite.

The macOS application is now **signed and notarized**. Install it from the DMG and open it normally, without the `xattr` workaround. Signing covers the bundled native code, and both the app and disk image have passed notarization and Gatekeeper checks.

### Colors and their priorities, rebuilt

We've completely reworked how colors are stored, assigned, and resolved. Objects and selections now follow the same rules, and those rules apply across commands, the interface, rendering, and saved scenes.

`color` changes base atom colors. An explicit representation color, such as `cartoon_color` or `surface_color`, takes priority for that representation. Later assignments replace earlier ones on the atoms they target. Changing the base color leaves explicit representation colors intact; `unset` removes the override and restores inherited coloring.

Coloring also does less repeated work: selectors are compiled once, palette and scheme lookups are cached, and whole-object assignments use a direct bulk update. Scenes preserve representation colors, and legacy PRS colors and imported PSE overrides are converted to the new model.

### Dragging, starting with labels

We've added shared drag handling to the native and web viewers. Annotation labels are the first objects to use it; we can extend the same behavior to other objects later.

Hold **Cmd+Shift on macOS** or **Ctrl+Shift on Windows and Linux** and drag a label in the viewport. Its screen offset is saved in the PRS session, so the placement survives reopening the file.

## Configuration and compatibility

Application preferences and the plugin list now share **`config.toml`**. The REPL defaults to the bottom; use `repl_position = "top"` to move it to the top.

**`plugins.toml` is no longer read.** Close Patinae and use the supplied [migration script](https://github.com/zmactep/patinae/blob/v0.5.2/docs/migrate-plugins-config.md) to transfer an existing plugin list, then restart the application.

Newly saved sessions use **PRS format 8**; reopen them with 0.5.2. Plugin ABI 7 and runtime wire version 21 are unchanged, but native plugins should be built from matching 0.5.2 sources because the internal color and session representations have changed.

---

## Other improvements

- The sequence viewer draws only visible rows and a small overscan margin. In a local 3J3Q check, viewer color toggles dropped from roughly 1.7–2.1 seconds to 225–238 milliseconds.
- Structure loading shares residue metadata and label strings. mmCIF and BinaryCIF imports retain label residue identifiers and full polymer sequences, including unresolved residues.
- Automatic memory profiles use the active GPU's native memory budget where available. On macOS, balanced and lite profiles also reduce allocator cache retention at startup.
- GPU timing no longer resolves unwritten timestamp queries. This fixes a Vulkan validation error; the reported NVIDIA/Vulkan device-loss issue still needs confirmation on the affected hardware.

---

## Downloads

### Bundles

Packages with the application, packaged plugins, and runtime pieces.

| Platform | Architecture | Download |
| --- | --- | --- |
| macOS | Apple Silicon | [Patinae.dmg](https://github.com/zmactep/patinae/releases/download/v0.5.2/Patinae.dmg) |
| Windows | x86_64 | [Windows bundle](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-bundle-windows-x86_64.zip) |
| Windows | ARM64 | [Windows ARM64 bundle](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-bundle-windows-arm64.zip) |

### Standalone executables

| Platform | Architecture | Executable | Plugins |
| --- | --- | --- | --- |
| macOS | Apple Silicon | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-macos-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-plugins-macos-arm64.tar.gz) |
| Windows | x86_64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-windows-x86_64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-plugins-windows-x86_64.zip) |
| Windows | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-windows-arm64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-plugins-windows-arm64.zip) |
| Linux | x86_64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-linux-x86_64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-plugins-linux-x86_64.tar.gz) |
| Linux | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-linux-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-plugins-linux-arm64.tar.gz) |

The AI plugin currently requires a separate source build; the release workflow does not yet include it in plugin archives. The local `make plugins` target does include it. Follow the [AI plugin guide](https://github.com/zmactep/patinae/blob/v0.5.2/plugins/ai/README.md) to install the library and create its configuration.

### Python and web

| Package | Download |
| --- | --- |
| Python wheels | [PyPI package](https://pypi.org/project/patinae/) or release assets |
| Web viewer (WASM + JS) | [patinae-web.tar.gz](https://github.com/zmactep/patinae/releases/download/v0.5.2/patinae-web.tar.gz) |

---

**Full Changelog:** [v0.5.1 → v0.5.2](https://github.com/zmactep/patinae/compare/v0.5.1...v0.5.2)
