# Patinae plugins

Native plugins extend the Patinae desktop application with commands, panels,
scripting, rendering, and external control.

| Plugin | Cargo package | What it adds |
| --- | --- | --- |
| [Hello](hello/README.md) | `hello-plugin` | Minimal command and settings example |
| [Raytracer](raytracer/README.md) | `raytracer-plugin` | `ray`, GPU ray tracing, PNG export, and a rendering panel |
| [IPC](ipc/README.md) | `ipc-plugin` | Local Unix socket control for external clients |
| [Python](python/README.md) | `python-plugin` | Embedded Python, `.py` scripts, atom expressions, and an editor panel |
| [AI](ai/README.md) | `ai-plugin` | Natural-language control with scene images and optional MCP tools |

## Build and load

Run build commands from the repository root. Build plugins and the application
from the same source revision: the native plugin ABI and runtime wire contracts
must match. Rust and Cargo are required; building all plugins also requires the
[Python build prerequisites](python/README.md#build-and-install).

```sh
make patinae
make plugins
./target/release/patinae
```

`make plugins` stages the libraries in `target/release/plugins/`, where the
release executable discovers them. The Windows build excludes IPC.

On macOS and Linux, install into the user plugin directory with:

```sh
make plugins-install
```

The default installation destination is `~/.patinae/plugins/`. To install into a
different directory, set `PLUGIN_INSTALL_DIR` explicitly:

```sh
make plugins-install PLUGIN_INSTALL_DIR=/absolute/path/to/plugins
```

For an individual plugin, follow its README. Cargo library names use underscores:
`libhello_plugin.dylib` on macOS, `libhello_plugin.so` on Linux, and
`hello_plugin.dll` on Windows. Copy the library into the plugin directory itself.

## Discovery and verification

The user plugin directory is selected in this order:

1. `PATINAE_PLUGIN_DIR`, when set.
2. `<PATINAE_CONFIG_DIR>/plugins`, when the configuration directory is overridden.
3. `~/.patinae/plugins/` otherwise.

Patinae also searches application-relative plugin locations, including
`plugins/` beside the executable and `Contents/PlugIns/` in a macOS app bundle.
The Makefile installation destination is separate from these runtime environment
variables; use `PLUGIN_INSTALL_DIR` when installing to a custom location.

Restart Patinae after installing or rebuilding a library. In the Patinae REPL:

```pml
capabilities plugins
capabilities settings
capabilities formats run
```

`capabilities` reports loaded plugins and active handlers. Use `help hello`,
`help ray`, `help python`, or `help ai` for command syntax. IPC adds no REPL
command and needs `PATINAE_IPC_SOCKET` to start its server. AI needs a separate
configuration file; installing the library does not create one.

If a plugin is missing, check the startup log, library location, platform and
architecture, and that the host and plugin were rebuilt together. Python also
needs its matching runtime available when the library loads.

## Develop a plugin

Start with the [plugin authoring guide](../docs/make-your-own-plugin.md).
[Hello](hello/README.md) is the smallest implementation; the other plugins show
panels, background tasks, renderer integration, and external command execution.

These dynamic libraries target native hosts. For browser integration, see the
[web viewer](../web/README.md); for scripts and notebooks outside the desktop
application, see the [Python package](../python/README.md).
