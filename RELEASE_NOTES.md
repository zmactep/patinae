# Patinae v0.5.4

Since 0.5.3, I've spent most of my time on the documentation. The new lessons run Patinae directly in the browser, so that's where most of the fixes went. You can now finish a lesson by downloading the scene you built, the atoms you selected, or a figure at the size you need.

## Highlights

### Interactive lessons

The redesigned homepage shows the same protein in four representations. Drag any of the four views to rotate them together. [Patinae homepage](https://patinae.bio/)

**Quick start** walks through opening hemoglobin, moving the camera, changing representations and colors, and saving your work. Click **Run** beside a command to see what it does to the structure. The final exercise gives you a second hemoglobin structure to explore on your own. [Quick start](https://patinae.bio/docs/quick-start.html)

**Selection algebra** starts with a heme group. First, select the protein atoms within 4 Å of the heme. Then expand the selection to whole residues and compare the two views. The lesson ends with an exercise in finding a heme pocket in 4HHB and exporting its coordinates. [Selection algebra](https://patinae.bio/docs/selections.html)

### Save files from the web viewer

Run `save hemoglobin.prs` in the browser console to download an editable session you can reopen in desktop Patinae. To export coordinates, give `save` a structure filename and a selection. For example, `save chainA.cif.gz, chain A` writes just chain A to a compressed mmCIF file. Both commands also work in PML scripts.

Use `png hemoglobin.png, 1600, 1200` to render a 1600 × 1200 image, including labels, without resizing the viewer. Supply just the width to preserve the current aspect ratio. For a transparent background, run `set opaque_background, 0` before exporting.

If you embed Patinae in a web page, `viewer.exportFile()` and `viewer.exportPng()` return a `Blob` you can use directly in your application.

### Web viewer startup

We fixed a regression that could leave the viewer waiting indefinitely during startup. The renderer also now needs only eight storage buffers per shader stage, allowing it to start on WebGPU devices that expose eight or nine.

## Other improvements

- **Line width** can now be set globally or per object. Use `set line_width, 3` for three-pixel bonds in the lines representation. The default is 1 physical pixel, and the range is 1–20.
- **Thin sticks** render and pick more reliably in orthographic views and when the camera is far from the structure. This fixes gaps that could appear along a bond.
- **Desktop Python panel.** Resizing the bottom dock now gives both the input and output editors more room.
- The new render memory guide explains how Patinae selects a GPU memory profile, what each profile changes, and how to override it. [Render memory profiles](https://github.com/zmactep/patinae/blob/v0.5.4/docs/render-memory-profiles.md)
- Contributor setup now lives in a separate guide. `make test-ci` runs the full set of local checks and reports failures together. [Contributor guide](https://github.com/zmactep/patinae/blob/v0.5.4/CONTRIBUTING.md)

## Configuration and compatibility

Patinae 0.5.4 uses PRS format 8, plugin ABI 7, and runtime wire version 21. Sessions saved in 0.5.3 open directly in 0.5.4. Build native plugins from matching 0.5.4 sources.

## Downloads

### Full packages

| Platform | Architecture | Installer | Portable bundle |
| --- | --- | --- | --- |
| macOS | Apple Silicon | [Patinae.dmg](https://github.com/zmactep/patinae/releases/download/v0.5.4/Patinae.dmg) | — |
| Windows | x64 | [Windows installer](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-setup-windows-x86_64.exe) | [Windows bundle](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-bundle-windows-x86_64.zip) |
| Windows | ARM64 | [Windows ARM64 installer](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-setup-windows-arm64.exe) | [Windows ARM64 bundle](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-bundle-windows-arm64.zip) |

### Standalone executables

| Platform | Architecture | Executable | Plugins |
| --- | --- | --- | --- |
| macOS | Apple Silicon | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-macos-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-plugins-macos-arm64.tar.gz) |
| Windows | x64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-windows-x86_64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-plugins-windows-x86_64.zip) |
| Windows | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-windows-arm64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-plugins-windows-arm64.zip) |
| Linux | x86_64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-linux-x86_64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-plugins-linux-x86_64.tar.gz) |
| Linux | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-linux-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-plugins-linux-arm64.tar.gz) |

### Python and web

| Package | Download |
| --- | --- |
| Python wheels | [PyPI package](https://pypi.org/project/patinae/) or release assets |
| Web viewer (WASM + JS) | [patinae-web.tar.gz](https://github.com/zmactep/patinae/releases/download/v0.5.4/patinae-web.tar.gz) |

---

**Full Changelog:** [v0.5.3 → v0.5.4](https://github.com/zmactep/patinae/compare/v0.5.3...v0.5.4)
