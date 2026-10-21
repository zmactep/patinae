# Patinae plugins

Plugins add commands and tools to the Patinae desktop app.

| Plugin | What you can do |
| --- | --- |
| [Hello](hello/README.md) | Try a minimal plugin with a `hello` command |
| [Raytracer](raytracer/README.md) | Render a scene with GPU ray tracing and export a PNG |
| [Python](python/README.md) | Run Python commands and scripts inside Patinae |
| [AI](ai/README.md) | Control Patinae using natural language |
| [IPC](ipc/README.md) | Control a running Patinae app from another program |

Each plugin's README has its setup instructions and usage examples. Python
needs a matching Python runtime, AI needs a provider configuration, and IPC
needs a socket path before it can accept connections.

## Install a plugin

Copy the plugin library into `~/.patinae/plugins/` and restart Patinae.
The library extension depends on your platform: `.dylib` on macOS, `.so` on
Linux, or `.dll` on Windows. Use a plugin built for your platform and the same
Patinae version or source revision as the app.

In the Patinae command bar, check which plugins loaded:

```pml
capabilities plugins
```

For example, with Hello installed, enter `hello Alice` to try it or `help hello`
to see its command syntax.

## Build the bundled plugins

You need Rust and Cargo. Building the full set also requires the
[Python build prerequisites](python/README.md#build-and-install).
Run these commands from the repository root:

```sh
make patinae
make plugins
./target/release/patinae
```

The plugins are placed in `target/release/plugins/`, ready for the app to load.
IPC is excluded on Windows. To build just one plugin, follow its README.

On macOS and Linux, `make plugins-install` builds and copies the bundled plugins
to `~/.patinae/plugins/`. For another destination, use
`make plugins-install PLUGIN_INSTALL_DIR=/absolute/path/to/plugins`.

## Choose which plugins to load

By default, Patinae finds plugins automatically. Use the **Plugins** dialog to
add or remove libraries, then click **Save** to apply the changes.

You can also edit `~/.patinae/config.toml`. For example, to load only Hello on
macOS, add:

```toml
[[plugin]]
path = "plugins/libhello_plugin.dylib"
```

Add one `[[plugin]]` entry per library, in the order you want them loaded.
Use the appropriate filename for your platform. Paths are relative to the
folder containing `config.toml`; absolute paths also work.

To disable all plugins, replace the list with `plugin = []`. To restore
automatic discovery, remove the `plugin` list entirely. Restart Patinae after
editing the file by hand.

For older installations, see [Move an old plugin list to config.toml](../docs/migrate-plugins-config.md).

### Custom directories

`PATINAE_CONFIG_DIR` changes the configuration folder; Patinae then reads
`config.toml` and looks in `plugins/` inside that folder. `PATINAE_PLUGIN_DIR`
overrides the user plugin folder alone. Patinae also searches `plugins/` next
to the executable and `Contents/PlugIns/` in macOS app bundles.

An explicit list in `config.toml` replaces automatic discovery. The Makefile
uses `PLUGIN_INSTALL_DIR` for installation, independently of these runtime
settings.

## If a plugin does not load

Check the startup log for the library path and error. If you have a plugin list
in `config.toml`, make sure the library is included. A configuration error stops
plugin loading until the file is fixed.

For a library compatibility error, rebuild the app and plugin from the same
source revision, then restart. For Python, check that its matching runtime is
available; see the [Python setup instructions](python/README.md#build-and-install).

## Write your own plugin

Follow the [plugin authoring guide](../docs/make-your-own-plugin.md) to build
your first plugin. [Hello](hello/README.md) is the smallest working example;
the other plugins demonstrate panels, scripting, rendering, and external control.
