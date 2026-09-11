# Make your own Patinae plugin

Patinae plugins are native Rust dynamic libraries that the desktop app loads at
startup. A plugin can add REPL commands, dynamic settings, script and file-format
handlers, docked GUI panels, background workers, viewport image overlays, and
renderer-adjacent GPU work.

Keep language runtimes and specialized behavior inside the plugin. The core
owns shared command, task, scene and rendering contracts; plugins use those
contracts through the SDK instead of adding plugin-specific branches to the core.

This guide is written as a tutorial first: by the end of the first half you will
have a working external `hello` plugin that Patinae can load. The later sections
are compact maps for the optional surfaces you can add after the first plugin is
working.

Reference implementations in this repository:

| Reference | Use it when you need |
| --- | --- |
| `plugins/hello` | The smallest command and settings example |
| `plugins/python` | Script handling, bottom panels, workers, and poll loops |
| `plugins/raytracer` | Settings, panels, renderer artifacts, GPU callbacks, and viewport images |
| `plugins/ipc` | Headless message handling and external control integration |

## What you will build

You will create an external Rust crate named `my-patinae-plugin`. It will build
to a shared library, Patinae will discover it at startup, and the REPL command
below will print a message:

```text
hello Alice
```

Expected result in the Patinae output panel:

```text
Greetings from plug-in system, Alice!
```

You need:

- A Patinae desktop build or installed Patinae binary.
- Rust and Cargo.
- A Patinae SDK dependency source. Use the same `0.4.0` release, Git tag, or Git
  revision for every Patinae crate in the plugin.

## Create an external plugin crate

Keep plugin projects outside the Patinae workspace. This gives your plugin the
same shape users will have: a normal Rust crate depending on Patinae as an SDK,
not a workspace member using local path dependencies.

```bash
cargo new --lib my-patinae-plugin
cd my-patinae-plugin
```

Replace `Cargo.toml` with a `cdylib` manifest. A Git-pinned dependency is the
most explicit choice while developing against a specific Patinae release:

```toml
[package]
name = "my-patinae-plugin"
version = "0.1.0"
edition = "2021"
license = "BSD-3-Clause"

[lib]
crate-type = ["cdylib"]

[dependencies]
patinae-plugin = { git = "https://github.com/zmactep/patinae", tag = "v0.4.0" }
```

If the SDK crates are published for your target release, the equivalent
crates.io dependency is:

```toml
[dependencies]
patinae-plugin = "0.4.0"
```

Do not mix SDK sources. For example, do not combine `patinae-plugin = "0.4.0"`
with `patinae-render` from a different Git revision. Mixed revisions can compile
incompatible wire types into the plugin.

## Add the first command

A plugin starts with the SDK prelude, the `patinae_plugin!` declaration macro,
and one or more types implementing `Command`.

Replace `src/lib.rs` with:

```rust
use patinae_plugin::patinae_plugin;
use patinae_plugin::prelude::*;

struct HelloCommand;

impl Command for HelloCommand {
    fn name(&self) -> &str {
        "hello"
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let name = args.get_str(0).unwrap_or("World");
        ctx.print(&format!("Greetings from plug-in system, {}!", name));
        Ok(())
    }

    command_help! {
        CMD "hello"
        DESCRIPTION ["greets the user from the plugin system."]
        REQUIRED []
        OPTIONAL [
            { "name", "string", "name to greet", "World" },
        ]
        EXAMPLES ["hello", "hello Alice"]
    }

    fn arg_hints(&self) -> &[ArgHint] {
        &[ArgHint::None]
    }
}

patinae_plugin! {
    name: "hello",
    description: "Example plugin: registers a hello command",
    commands: [HelloCommand],
}
```

The `patinae_plugin!` macro creates the exported ABI declaration loaded by the
host. If you do not pass `version: "..."`, the macro uses the plugin crate's
`CARGO_PKG_VERSION`.

Command callbacks should return normal `CmdResult` errors. The macro protects
registration with `catch_unwind`, but runtime code should not rely on panics for
control flow.

## Build the plugin

Build the crate in release mode:

```bash
cargo build --release
```

Expected result: Cargo finishes the release build and writes one shared library
under `target/release`. Cargo replaces hyphens with underscores in the library
file name:

| Platform | File name |
| --- | --- |
| macOS | `target/release/libmy_patinae_plugin.dylib` |
| Linux | `target/release/libmy_patinae_plugin.so` |
| Windows | `target\release\my_patinae_plugin.dll` |

The file extension matters. Patinae discovers `.dylib` on macOS, `.so` on Linux,
and `.dll` on Windows.

## Install and verify

Patinae loads plugins at startup. Copy the built library into the user plugin
directory, then restart Patinae.

In the default desktop setup the user plugin directory resolves to
`~/.patinae/plugins`. You can override it with `PATINAE_PLUGIN_DIR`. If
`PATINAE_CONFIG_DIR` is set, the default plugin directory becomes
`<config_dir>/plugins`. Packaged builds also check a `plugins` directory beside
the executable, and macOS bundles additionally check `Contents/PlugIns`.

macOS and Linux:

```bash
mkdir -p ~/.patinae/plugins
```

macOS:

```bash
cp target/release/libmy_patinae_plugin.dylib ~/.patinae/plugins/
```

Linux:

```bash
cp target/release/libmy_patinae_plugin.so ~/.patinae/plugins/
```

Windows PowerShell:

```powershell
New-Item -ItemType Directory -Force "$env:USERPROFILE\.patinae\plugins"
Copy-Item target\release\my_patinae_plugin.dll "$env:USERPROFILE\.patinae\plugins\"
```

For a faster development loop, point Patinae at Cargo's release output:

```bash
PATINAE_PLUGIN_DIR="$PWD/target/release" patinae
```

Restart Patinae after every rebuild. Plugins are loaded before startup scripts
and before files from the command line are opened, so plugin commands, script
handlers, and format handlers are available during startup workflows.

After restart, first verify that the plugin completed loading and registration:

```text
capabilities plugins
```

For the tutorial plugin, the relevant output is:

```text
Loaded plugins:
hello	0.1.0	Example plugin: registers a hello command
```

The three fields are separated by tab characters. This command lists only
successfully loaded plugin instances retained in the current process, using
their declared name, version, and description. It does not list failed plugins
or every library found on disk. If no plugin was retained, the fixed body is
`(none)`.

Then verify the registered command:

```text
hello Alice
```

Expected result:

```text
Greetings from plug-in system, Alice!
```

The Patinae repository targets `make plugins`, `make plugins-install`, and
`make patinae-fast-plugins` are for the bundled reference plugins in
`plugins/*`. External plugin projects do not need those targets.

## Read command arguments and settings

Most useful plugins need either arguments, settings, or both. Keep command
logic simple: parse user input with `ParsedCommand`, and read settings through
the command context instead of manually touching registries.

Useful command helpers:

| Need | API |
| --- | --- |
| Positional strings, integers, floats, booleans | `get_str`, `get_int`, `get_float`, `get_bool` |
| Named strings, integers, floats, booleans | `get_named_str`, `get_named_int`, `get_named_float`, `get_named_bool` |
| Output messages | `CommandContext::print`, `print_warning`, `print_error` |
| Host actions | `show_panel`, `hide_panel`, `clear_output`, `quit`, `record_recent_file` |
| Viewer access | `ctx.viewer`, a `ViewerLike` handle |

Commands use `ArgumentSyntax::Pml` by default. If your command interprets its
own language, override `Command::argument_syntax()` to return
`ArgumentSyntax::Verbatim`. The host passes the rest of the logical line as one
string argument and as `ParsedCommand::raw_args()`, preserving semicolons,
quotes and explicit line continuations. The declaration applies to aliases,
ordinary command execution and PML scripts; no language-name dispatch is needed
in the host. This requires the current SDK/host ABI; rebuild both from matching
sources when changing the descriptor contract.

Define plugin settings with `define_plugin_settings!`:

```rust
use patinae_plugin::{define_plugin_settings, patinae_plugin};
use patinae_plugin::prelude::*;

define_plugin_settings! {
    HelloSettings {
        style: i32 = 0, name = "hello_style";
        enabled: bool = true, name = "hello_enabled";
        scale: f32 = 1.0, name = "hello_scale",
            min = 0.1, max = 10.0,
            side_effects = [SceneInvalidate];
    }
}

patinae_plugin! {
    name: "hello",
    description: "Example settings plugin",
    commands: [HelloCommand],
    settings: [HelloSettings],
}
```

Users can then run normal setting commands:

```text
set hello_style, 1
set hello_enabled, off
set hello_scale, 1.5
get hello_style
```

Read the current values in command code through typed context helpers:

```rust
#[derive(Debug, Clone, Copy)]
struct HelloRuntimeSettings {
    style: i32,
    enabled: bool,
    scale: f32,
}

impl HelloRuntimeSettings {
    fn read<'v, 'r, V>(ctx: &CommandContext<'v, 'r, V>) -> Self
    where
        V: ViewerLike + ?Sized,
    {
        Self {
            style: ctx.setting_int("hello_style", 0),
            enabled: ctx.setting_bool("hello_enabled", true),
            scale: ctx.setting_float("hello_scale", 1.0),
        }
    }
}
```

Use that settings bag inside `execute()` before applying behavior:

```rust
impl Command for HelloCommand {
    fn name(&self) -> &str {
        "hello"
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        let settings = HelloRuntimeSettings::read(ctx);
        if !settings.enabled {
            ctx.print_warning("hello plugin is disabled");
            return Ok(());
        }

        let name = args.get_str(0).unwrap_or("World");
        let greeting = match settings.style {
            1 => "Hello",
            2 => "Hi",
            _ => "Greetings from plug-in system",
        };
        ctx.print(&format!(
            "{}: {}, scale = {}",
            greeting, name, settings.scale
        ));
        Ok(())
    }
}
```

`CommandContext::setting_value` and the typed `setting_*` helpers resolve
built-in settings first, then plugin dynamic settings. For plugin settings they
use the current dynamic store value, fall back to the descriptor default after
`unset`, and finally use the caller default if the setting is unknown or has an
incompatible type.

Panel code has the same read-only contract through
`SharedContext::setting_value`, `setting_int`, `setting_bool`, and
`setting_float`.

The raw dynamic setting registry and `DynamicSettingStore` are public SDK
contracts, but ordinary commands and panels should not lock the store directly.
Use the raw store only for registration, plugin-owned poll state, or host
plumbing. For many settings, put the typed settings bag in a `settings.rs`
module; `plugins/raytracer` uses this pattern before passing settings into
renderer code.

Common setting side effects include `SceneInvalidate`, `RepresentationRebuild`,
`ColorRebuild`, `ViewportUpdate`, and `FullRebuild`.

## Request runtime inputs explicitly

Plugin commands cross an ABI boundary. The host only prepares the expensive
runtime state that the command asks for, so declare the smallest requirement
that matches the job.

```rust
impl Command for GeometryStatsCommand {
    fn name(&self) -> &str {
        "geometry_stats"
    }

    fn runtime_requirements(&self) -> CommandRuntimeRequirements {
        CommandRuntimeRequirements::DISPLAYED_GEOMETRY
    }

    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        _args: &ParsedCommand,
    ) -> CmdResult {
        let options = patinae_render::GeometryExportOptions::default();
        let mut object_count = 0usize;
        ctx.viewer
            .for_each_displayed_geometry_chunk(&options, &mut |chunk| {
                object_count += chunk.objects.len();
                Ok(())
            })
            .map_err(CmdError::execution)?;

        ctx.print(&format!("Displayed objects: {}", object_count));
        Ok(())
    }
}
```

This snippet names `patinae_render` types directly. Add the dependency from the
same SDK source as `patinae-plugin`:

```toml
[dependencies]
patinae-plugin = { git = "https://github.com/zmactep/patinae", tag = "v0.4.0" }
patinae-render = { git = "https://github.com/zmactep/patinae", tag = "v0.4.0" }
```

Or, if using crates.io SDK crates:

```toml
[dependencies]
patinae-plugin = "0.4.0"
patinae-render = "0.4.0"
```

Important command requirements:

| Requirement | Use it for |
| --- | --- |
| `NONE` | Commands that only parse args, print, or call cheap viewer APIs |
| `FULL_SESSION` | Serialized scene/session state |
| `DISPLAYED_GEOMETRY` | Renderer-neutral visible primitives, possibly spooled if large |
| `TRACE_GEOMETRY_STREAM` | Compact ray/trace-friendly geometry chunks |
| `GPU_COMMANDS` | Host-executed portable GPU callbacks |
| `RENDER_ARTIFACTS` | Renderer-owned GPU artifact handles for the current frame |

`plugins/raytracer` requests
`GPU_COMMANDS.union(CommandRuntimeRequirements::RENDER_ARTIFACTS)` for the ray
command because it consumes renderer buffers and dispatches GPU work. It does
not request `FULL_SESSION`.

## Handle scripts and file formats

Use handlers when your plugin wants to join existing Patinae file-opening flows
instead of inventing a separate command.

Script handlers are for the `run` command. Register lowercase extensions
without a dot:

```rust
patinae_plugin! {
    name: "script-example",
    description: "Runs .foo scripts",
    commands: [],
    register: |reg| {
        reg.register_script_handler("foo", |path: &str| {
            Ok(AsyncCommandRequest::Plugin(PluginTaskRequest::new(
                "foo_script", serde_json::json!({"path": path}),
            )))
        });
    },
}
```

The plugin must install a polling message handler to receive accepted task
invocations. The host assigns the TaskId before the handler starts work; report
start, output and completion through `PollContext::report_task_event`. See
[Background tasks](#background-tasks) for the execution contract.

File format handlers are for `load` and `save`. They declare readable and/or
writable extensions:

```rust
use std::io::{Read, Write};
use std::sync::Arc;

use patinae_plugin::patinae_plugin;
use patinae_plugin::prelude::*;

patinae_plugin! {
    name: "format-example",
    description: "Adds a toy molecule-name format",
    commands: [],
    register: |reg| {
        reg.register_format_handler(FormatHandler {
            name: "Name List".into(),
            extensions: vec!["names".into()],
            reader: Some(Arc::new(|mut input| {
                let mut text = String::new();
                input.read_to_string(&mut text).map_err(|err| err.to_string())?;

                let molecules = text
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(ObjectMolecule::new)
                    .collect();

                Ok(molecules)
            })),
            writer: Some(Arc::new(|mut output, molecules| {
                for molecule in molecules {
                    writeln!(output, "{}", molecule.name).map_err(|err| err.to_string())?;
                }
                Ok(())
            })),
        });
    },
}
```

Built-in formats take priority. Plugin readers are used when `load file.ext`
matches a registered readable extension; plugin writers are used when
`save file.ext` matches a registered writable extension. Startup argv, menu
opens, and drag-and-drop route through the same native file-action path, so a
registered readable format is available through all file-open surfaces.

After rebuilding and restarting Patinae, verify the effective handlers in that
process:

```text
capabilities formats run
capabilities formats load
capabilities formats save
```

The script example above should add `.foo` under `Formats for run:`. The format
example should add `.names` under both `Formats for load:` and
`Formats for save:` because it supplies both a reader and a writer. A
reader-only handler appears only under `load`; a writer-only handler appears
only under `save`.

These leaves report normalized suffixes accepted by the actual current command
maps. Built-in handlers shadow plugin registrations for the same suffix, and a
collision between plugins reports only the current map winner. The output does
not identify providers or define plugin discovery precedence, so verify the
suffix and then exercise the matching command. Treat this runtime output as the
canonical format list instead of copying the full matrix into plugin
documentation.

## Add a docked GUI panel

Use a panel when the plugin needs persistent UI. Plugin panels are declarative:
the plugin returns a portable `PanelSnapshot`, and the frontend decides how to
render it.

```rust
struct MiniPanel {
    enabled: bool,
    radius: f32,
}

impl MiniPanel {
    fn new() -> Self {
        Self {
            enabled: true,
            radius: 0.25,
        }
    }
}

impl PluginPanel for MiniPanel {
    fn descriptor(&self) -> PanelDescriptor {
        PanelDescriptor::right("mini_panel", "Mini")
            .icon("M")
            .default_visible(false)
    }

    fn runtime_requirements(&self) -> PanelRuntimeRequirements {
        PanelRuntimeRequirements::NONE
    }

    fn snapshot(&mut self, _ctx: &SharedContext<'_>) -> PanelSnapshot {
        PanelSnapshot::new(vec![
            PanelControl::Heading {
                id: "title".into(),
                text: "Mini Plugin".into(),
            },
            PanelControl::Toggle {
                id: "enabled".into(),
                label: "Enabled".into(),
                value: self.enabled,
            },
            PanelControl::Slider {
                id: "radius".into(),
                label: "Stick radius".into(),
                value: self.radius,
                min: 0.05,
                max: 1.0,
                step: 0.05,
            },
            PanelControl::Button {
                id: "apply".into(),
                label: "Apply".into(),
                primary: true,
            },
        ])
    }

    fn handle_event(
        &mut self,
        event: PanelEvent,
        _ctx: &SharedContext<'_>,
        _bus: &mut MessageBus,
    ) -> Vec<PanelAction> {
        match event.control_id.as_str() {
            "enabled" => {
                if let PanelValue::Bool(value) = event.value {
                    self.enabled = value;
                }
                Vec::new()
            }
            "radius" => {
                if let PanelValue::Number(value) = event.value {
                    self.radius = value;
                }
                Vec::new()
            }
            "apply" => vec![PanelAction::ExecuteCommand {
                command: format!("set stick_radius, {}", self.radius),
                silent: false,
            }],
            _ => Vec::new(),
        }
    }
}

patinae_plugin! {
    name: "mini-panel",
    description: "Shows a small plugin panel",
    commands: [],
    panels: [MiniPanel::new()],
}
```

Common controls: `Text`, `Heading`, `Spacer`, `Button`, `ButtonRow`, `Toggle`,
`Slider`, `Number`, `Select`, `TextInput`, `TextArea`, `Row`, `Column`, `Group`,
and `Image` with RGBA bytes.

Common actions: `ExecuteCommand { command, silent }`,
`SetSetting { name, value }`, and `Custom { topic, payload }`.

Use `PanelRuntimeRequirements::NONE` for panels that only render plugin-owned
state. Use `FULL_SESSION` only when the panel needs a serialized scene snapshot.
Most controls should update plugin-owned panel state in `handle_event`; use
actions when the host needs to execute a command, set a setting, or dispatch a
custom topic.

For richer examples, see `plugins/python/src/panel.rs` for a bottom script
editor with `TextArea` highlights, and `plugins/raytracer/src/panel.rs` for a
right dock panel that edits dynamic settings and requests a save dialog.

## Poll in the background

Use `MessageHandler` for headless behavior and per-frame polling: workers,
sockets, dynamic commands, notifications, host queries, deferred viewer
mutations, and plugin hotkeys.

```rust
struct BackgroundWorker {
    registered: bool,
}

impl MessageHandler for BackgroundWorker {
    fn on_message(&mut self, _msg: &AppMessage, _bus: &mut MessageBus) {}

    fn needs_poll(&self) -> bool {
        true
    }

    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        if !self.registered {
            ctx.register_dynamic_command(
                "mini_zoom".into(),
                "Zoom through a plugin dynamic command".into(),
                "mini_zoom".into(),
                String::new(),
            );
            self.registered = true;
        }

        for invocation in ctx.dynamic_invocations {
            if invocation.name == "mini_zoom" {
                ctx.execute_command(1, "zoom", false);
            }
        }
    }
}

patinae_plugin! {
    name: "background-example",
    description: "Registers a polling handler",
    commands: [],
    register: |reg| {
        reg.set_message_handler(BackgroundWorker { registered: false });
    },
}
```

Each poll includes these string lists in `ctx.poll_shared`:

- `object_names`: loaded object names in scene order.
- `selection_names`: all named selections in lexicographic order, including hidden,
  empty, and special selections such as `indicate`. Generated `pkN` aliases are not
  added to this list.
- `pick_paths`: unmodified canonical paths from Recent Atoms in list order,
  including any instance qualifiers. The first entry corresponds to `pk1`, the
  second to `pk2`; removing an entry shifts subsequent indices.

An empty list means there are no entries. These lists copy names and paths without
evaluating selections or serializing the full session.

Important `PollContext` methods:

| Need | API |
| --- | --- |
| Queue a host command | `execute_command(id, command, silent)` |
| Expose commands backed by poll logic | `register_dynamic_command`, `unregister_dynamic_command` |
| Show transient status | `set_notification` |
| Manage plugin hotkeys | `register_hotkey`, `unregister_hotkey` |
| Mutate the viewer on the host thread | `queue_viewer_mutation` |
| Update plugin viewport pixels | `set_viewport_image`, `clear_viewport_image`, `request_redraw` |
| Refresh plugin UI | `request_panel_update` |

Keep `poll()` non-blocking. The Python plugin uses a worker thread and polling
to transfer results back into the app without freezing the render loop.

### Background tasks

For command-owned background work, return
`ctx.request_task(AsyncCommandRequest::Plugin(request))` with a
`PluginTaskRequest`; script handlers return the request directly. Start the
worker only after the host admits the request and delivers
`TaskInvocation { task_id, request }` in `ctx.task_invocations`. The host assigns
the executor identity and TaskId. Keep work queues in the plugin and task state
in the host's runner.

| Worker action | `PollContext` API |
| --- | --- |
| Report start, progress, output or completion | `report_task_event(id, task_id, event)` |
| Execute a command belonging to the task | `execute_task_command(id, task_id, command, silent)` |
| Request an acknowledged scene mutation | `apply_task_action(id, task_id, action)` |
| Observe a task or request cancellation | `get_task`, `list_tasks`, `wait_task`, `cancel_task` |
| Read the correlated response during a later poll | `task_result(id)` |

Use unique plugin-local IDs for outstanding queries; these correlate replies
and are separate from TaskIds. Wait for mutation acknowledgements before
reporting success. The host checks ownership and scene validity before applying
an action; atom-property batches are validated completely before the first write.
Preserve structured error codes when forwarding a failure.

Process `ctx.task_cancellations` cooperatively and report a cancelled outcome
after the worker stops. Cancelling does not undo effects already applied. A wait
timeout stops only the wait. Child tasks created through `execute_task_command`
belong to the parent; the parent remains active until they finish, and a child
failure fails it. Waiting for yourself, an ancestor or blocked work on the same
serial executor is rejected with `would_deadlock`.

Report small results through `TaskOutcome`, including diagnostics and the actual
effects (`none`, `applied`, `partial`, `unknown`). Successful completion means
required mutations were applied. Terminal outcomes are immutable, but retained
history is bounded: handle `expired` and `not_found` when observing old tasks.
See [Receive results of host commands](#receive-results-of-host-commands) for
command receipts and accepted task IDs.

## Work with rendering and geometry

Patinae exposes several renderer-adjacent surfaces. Pick the smallest one that
matches the plugin's job.

| Goal | Contract |
| --- | --- |
| Show generated RGBA pixels in the viewport | `ViewerLike::set_viewport_image` |
| Export visible primitives without renderer buffer access | `DISPLAYED_GEOMETRY` |
| Stream compact ray/trace primitives | `TRACE_GEOMETRY_STREAM` |
| Consume renderer-owned GPU buffers for the current frame | `RENDER_ARTIFACTS` |
| Dispatch portable host GPU work | `GPU_COMMANDS` |

### Viewport image overlay

Use this when your plugin already has RGBA pixels and wants Patinae to show them
in the viewport. The overlay persists until camera or scene changes clear it.

If you name `ViewportImage` through `patinae_scene`, add a matching
`patinae-scene` dependency:

```toml
[dependencies]
patinae-plugin = { git = "https://github.com/zmactep/patinae", tag = "v0.4.0" }
patinae-scene = { git = "https://github.com/zmactep/patinae", tag = "v0.4.0" }
```

```rust
use patinae_scene::ViewportImage;

ctx.viewer.set_viewport_image(Some(ViewportImage {
    data: image_data,
    width,
    height,
}));
```

### Read or modify a full session

`CommandRuntimeRequirements::FULL_SESSION` requests a complete session snapshot.
It remains the default for `Command::runtime_requirements()`. Read that snapshot
through `ctx.viewer.session()` or other immutable viewer accessors:

```rust
let count = ctx.viewer.session().registry.len();
ctx.print(&format!("Objects: {count}"));
```

Immutable access does not return a replacement session. Inspection therefore
preserves host object identity, dirty flags, state generations, and scene
invalidation state. The full input snapshot still incurs serialization and
transfer costs and remains subject to the runtime payload limit.

To modify the session, use the existing mutable accessors (`session_mut()`,
`objects_mut()`, `settings_mut()`, and the other sub-manager accessors) or viewer
mutation methods. The SDK treats mutable access as a request to write back the
session, even when the value is unchanged. On success, the host applies the
returned session as a whole. Existing modifying commands need no source changes.
Use immutable accessors for inspection, even when a mutable context is available.

At the wire boundary, an empty `WireCommandOutput.session` byte vector means
**no replacement**. A serialized empty `Session` means **replace with an empty
session**. The host applies a nonempty replacement only when the command requested
`FULL_SESSION` and returned `Ok(())`. On `Err`, the SDK omits the replacement and
the host ignores any session bytes, including malformed ones, while preserving
command diagnostics. A malformed replacement on success fails without replacing
the session.

Commands without `FULL_SESSION` continue to receive a lightweight snapshot and
cannot replace the host session through the command response. Viewport images
and GPU callbacks retain their separate behavior. This is not transaction support
for effects already performed through host callbacks or external resources.

### Displayed geometry

Use `DISPLAYED_GEOMETRY` for renderer-neutral visible primitives: meshes,
analytic spheres and cylinders, line segments, and point samples. It is a good
fit for exporters, debug tools, and offline analysis.

```rust
fn runtime_requirements(&self) -> CommandRuntimeRequirements {
    CommandRuntimeRequirements::DISPLAYED_GEOMETRY
}

let options = patinae_render::GeometryExportOptions::default();
ctx.viewer
    .for_each_displayed_geometry_chunk(&options, &mut |chunk| {
        for object in &chunk.objects {
            patinae_plugin::log::debug!(
                "object {} has {} primitives",
                object.object_id.0,
                object.primitives.len()
            );
        }
        Ok(())
    })
    .map_err(CmdError::execution)?;
```

Use the chunk visitor form for large scenes. Displayed geometry can be delivered
inline or through a process-local spool.

### Trace geometry stream

Use `TRACE_GEOMETRY_STREAM` when you want ray/trace-style chunks without pulling
the whole displayed scene into one ABI payload.

```rust
fn runtime_requirements(&self) -> CommandRuntimeRequirements {
    CommandRuntimeRequirements::TRACE_GEOMETRY_STREAM
}

let options = patinae_render::GeometryExportOptions::default();
ctx.viewer
    .for_each_trace_geometry_chunk(&options, &mut |chunk| {
        let count = chunk.spheres.len()
            + chunk.cylinders.len()
            + chunk.triangles.len()
            + chunk.line_segments.len()
            + chunk.point_samples.len();
        patinae_plugin::log::debug!("trace chunk: {} primitives", count);
        Ok(())
    })
    .map_err(CmdError::execution)?;
```

Trace chunks contain `TraceSphere`, `TraceCylinder`, `TraceTriangle`,
`TraceLineSegment`, and `TracePointSample`. Screen-space lines and points are
semantic samples; consumers choose how to map them to physical geometry.

### Renderer artifacts and GPU commands

Use `RENDER_ARTIFACTS` when you need renderer-owned GPU buffers for the current
displayed scene. Combine it with `GPU_COMMANDS` when the plugin also dispatches
host-executed GPU work.

```rust
fn runtime_requirements(&self) -> CommandRuntimeRequirements {
    CommandRuntimeRequirements::GPU_COMMANDS
        .union(CommandRuntimeRequirements::RENDER_ARTIFACTS)
}

ctx.viewer.prepare_render();

let snapshot = ctx
    .viewer
    .open_render_artifact_snapshot()
    .map_err(CmdError::execution)?;

ctx.print(&format!(
    "artifact layout {}, {} reps, {} buffers",
    snapshot.layout_version,
    snapshot.reps.len(),
    snapshot.buffers.len()
));

ctx.viewer
    .close_render_artifact_snapshot(snapshot.snapshot_id)
    .map_err(CmdError::execution)?;
```

Artifact descriptors expose roles such as `SceneColorLut`, `SphereInstances`,
`StickInstances`, `LineInstances`, `StdVertices`, `InstanceCount`, and
`IndirectDraw`. They are metadata plus opaque GPU handles, not borrowed
`wgpu::Buffer` references. Handles expire at the command boundary and should
not be cached by the plugin.

The portable GPU runtime is exposed through `ViewerLike` methods such as
`gpu_device_limits_for_plugins`, `gpu_create_buffer`, `gpu_write_buffer`,
`gpu_read_buffer`, `gpu_create_texture`, `gpu_create_cached_shader_module`,
`gpu_create_cached_compute_pipeline`, `gpu_create_bind_group`,
`gpu_dispatch_compute`, `gpu_submit_batch`, and `gpu_drop_handles`.

The plugin does not own the host `wgpu::Device`. It sends portable descriptors,
gets opaque handles back, and lets the host validate usage before touching
`wgpu`. Cached shader, layout, and pipeline calls are scoped by plugin and
device; temporary buffers and bind groups are command resources.

See `plugins/raytracer/src/artifact_gpu` for the advanced reference. It
validates layout roles and strides, creates command work buffers, uses cached
shader and pipeline handles, dispatches compute work, and reads back the final
image.

## Develop and validate

The tight loop for an external plugin is:

```bash
cargo check
cargo build --release
PATINAE_PLUGIN_DIR="$PWD/target/release" patinae
```

Restart Patinae after rebuilding. If you prefer copying into the user plugin
directory, use the install commands from "Install and verify".

Good validation targets for the plugin repository:

```bash
cargo check
cargo build --release
git diff --check
```

If you also maintain a Patinae source checkout, use `plugins/hello`,
`plugins/python`, and `plugins/raytracer` as reference implementations. The
Patinae workspace checks are useful for SDK changes, but an external plugin
should be validated from its own manifest.

## Choose the right contract

Use this map before adding a new plugin surface:

| Goal | Contract |
| --- | --- |
| Add a REPL command | `Command` plus `patinae_plugin! { commands: [...] }` |
| Add typed plugin settings | `define_plugin_settings!` plus `settings: [...]` |
| Open a custom script with `run` | `PluginRegistrar::register_script_handler` |
| Load or save a molecular file format | `FormatHandler` |
| Add a docked GUI surface | `PluginPanel` and declarative `PanelControl`s |
| Run a background worker | `MessageHandler::needs_poll` and `PollContext` |
| Execute host commands from a panel or poll loop | `PanelAction::ExecuteCommand` or `PollContext::execute_command` |
| Show generated pixels in the viewport | `ViewerLike::set_viewport_image` |
| Export visible primitives | `DISPLAYED_GEOMETRY` |
| Stream ray/trace primitives | `TRACE_GEOMETRY_STREAM` |
| Consume renderer GPU buffers | `RENDER_ARTIFACTS` |
| Dispatch host GPU work | `GPU_COMMANDS` |

Start with an isolated plugin crate, copy only the small reference pattern you
need from `plugins/hello`, and add one new surface at a time. Request only the
runtime inputs that surface needs.

## Troubleshooting

If Patinae does not load the plugin, check the shared library name first. Cargo
turns `my-patinae-plugin` into `my_patinae_plugin`, and each platform has a
different prefix and extension.

If the command is missing after a rebuild, restart Patinae. Plugins are loaded
only at startup.

If Patinae starts but does not discover the library, print or simplify your
plugin directory setup. The default is `~/.patinae/plugins`, but
`PATINAE_PLUGIN_DIR` replaces it, and `PATINAE_CONFIG_DIR` changes the derived
default to `<config_dir>/plugins`.

If the plugin compiles but fails to load or behaves strangely, make sure every
Patinae crate dependency uses the same version, tag, or revision.

If geometry, artifacts, or GPU calls return unavailable data, check the
command's `runtime_requirements()`. The host does not prepare expensive runtime
inputs unless the command requests them.

If a panel feels stale after plugin-owned state changes in `poll()`, call
`PollContext::request_panel_update`.

## Receive results of host commands

Call `PollContext::execute_command(id, command, silent)` to run an ordinary host
command after polling. A subsequent poll delivers a `CommandResult` only to the
requesting plugin, with the original `id`. IDs are local to each plugin; use a
distinct ID for each outstanding request within your plugin.

The result carries your correlation `id` and a shared `CommandReply` in `reply`.
The reply contains `result: Result<(), String>`, typed `messages` and accepted
`task_ids`. Field reads can use `response.result`, `response.messages` and
`response.task_ids` through `Deref`; construct values with `CommandResult::from_execution`
or `{ id, reply }`.

A successful reply is final only when `task_ids` is empty. For background work,
query or wait for each TaskId. A command failure may still contain accepted task
IDs; retain those IDs and avoid repeating the whole command. Message silence
controls presentation and does not discard the command receipt or task outcome.

```rust,ignore
fn poll(&mut self, ctx: &mut PollContext<'_>) {
    for response in ctx.command_results {
        self.record_receipt(response.id, &response.reply);
        for &task_id in &response.task_ids {
            self.observe_task(task_id);
        }
    }
}
```

Use `get_task`, `list_tasks`, `cancel_task`, `wait_task` and `task_result` for
observation through the existing correlated query transport. Producers return a
`PluginTaskRequest`; direct untracked worker submission is not a command result.
See [Background tasks](#background-tasks) for producer responsibilities.
