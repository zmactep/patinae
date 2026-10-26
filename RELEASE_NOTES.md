# Patinae v0.5.3

Patinae 0.5.3 focuses on Windows fixes: installation, the bundled Python runtime, and GPU resource limits. It also adds separate controls for bundled and user plugins and reduces the web viewer's download size.

## Highlights

### Windows installation and launch

Windows now has **full installers for x64 and ARM64**, alongside the portable ZIP bundles. Each includes the application, Python 3.13, and the Hello, Raytracer, Python, and AI plugins. No separate Python installation is needed; the AI plugin still requires a provider configuration.

A native launcher replaces the VBS script. The bundled Python runtime is now self-contained, removing its dependency on paths from the build machine. The launcher starts Patinae without a console window and forwards filenames containing spaces or Unicode characters.

Setup adds a Start menu shortcut, with optional desktop shortcuts and **Open with** entries. Existing default file associations are preserved. Later installers can upgrade the same installation, and uninstalling preserves user settings, user plugins, and documents.

### GPU limits and memory handling

Windows GPU initialization now negotiates buffer limits across the available hardware adapters while preserving selection of an adapter compatible with the application window.

Automatic memory profiles use the active GPU's available memory budget. A limit on the size of a single buffer no longer prevents an otherwise capable GPU from using the Performance profile.

Resource limits are checked in every profile, including Performance, before scene buffers are allocated or uploaded. Oversized requests are reported as errors, and recovery from out-of-memory conditions is preserved across redraws and scene updates.

### Bundled and user plugins

The Plugins dialog now separates **Bundled** and **User** libraries. Bundled plugins remain discoverable when a custom user plugin list is configured, and each has an enable switch. User plugins can still be added or removed from the list.

Click **Save** to apply changes without restarting. Disabled bundled plugins remain visible, and their settings survive moving the installation. Libraries listed in both groups are loaded only once.

## Other improvements

- PRS loading avoids an extra serialization and parsing pass while retaining format migrations and validation.
- The compressed WASM module is about **4% smaller** in the measured build.
- The web viewer stops retrying redraws after a hard GPU resource-limit error.
- Windows packaging fixes allow the Patinae wheel to be installed into the bundled runtime and correct validation of the launcher's Python search path.

## Configuration and compatibility

PRS format 8, plugin ABI 7, and runtime wire version 21 are unchanged. Build native plugins from matching 0.5.3 sources.

The Windows installer requires administrator privileges; ordinary application launches do not. Windows binaries and installers remain unsigned. IPC is not included on Windows.

## Downloads

### Full packages

| Platform | Architecture | Installer | Portable bundle |
| --- | --- | --- | --- |
| macOS | Apple Silicon | [Patinae.dmg](https://github.com/zmactep/patinae/releases/download/v0.5.3/Patinae.dmg) | — |
| Windows | x64 | [Windows installer](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-setup-windows-x86_64.exe) | [Windows bundle](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-bundle-windows-x86_64.zip) |
| Windows | ARM64 | [Windows ARM64 installer](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-setup-windows-arm64.exe) | [Windows ARM64 bundle](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-bundle-windows-arm64.zip) |

### Standalone executables

| Platform | Architecture | Executable | Plugins |
| --- | --- | --- | --- |
| macOS | Apple Silicon | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-macos-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-plugins-macos-arm64.tar.gz) |
| Windows | x64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-windows-x86_64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-plugins-windows-x86_64.zip) |
| Windows | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-windows-arm64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-plugins-windows-arm64.zip) |
| Linux | x86_64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-linux-x86_64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-plugins-linux-x86_64.tar.gz) |
| Linux | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-linux-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-plugins-linux-arm64.tar.gz) |

### Python and web

| Package | Download |
| --- | --- |
| Python wheels | [PyPI package](https://pypi.org/project/patinae/) or release assets |
| Web viewer (WASM + JS) | [patinae-web.tar.gz](https://github.com/zmactep/patinae/releases/download/v0.5.3/patinae-web.tar.gz) |

---

**Full Changelog:** [v0.5.2 → v0.5.3](https://github.com/zmactep/patinae/compare/v0.5.2...v0.5.3)
