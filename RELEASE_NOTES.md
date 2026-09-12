# Patinae v0.5.0

[Zawinski's law](https://en.wikiquote.org/wiki/Jamie_Zawinski) says that programs keep expanding until they can read email. Patinae started as a molecular viewer. It now has background tasks, Markdown output, and an AI plugin that can connect to MCP servers. We appear to be one mail server away from compliance.

The largest change in 0.5.0 is underneath all of that: **the task system has been completely rebuilt around one shared lifecycle**. Loading a structure, running a script, and asking a plugin to do background work now use the same rules for completion, progress, errors, and cancellation. Those rules extend across the desktop application, native plugins, IPC, Python, and the web viewer.

That gives plugins the foundation for longer workflows. A plugin can issue a command, receive its actual result, wait for any background work it starts, and decide what to do next. The new AI plugin puts that into practice: ask for a scene in ordinary language, let the agent operate Patinae, and have it inspect the rendered image before answering.

---

## Highlights

### A task has one lifecycle, wherever it starts

Background work now receives a host-assigned task ID and stays under the shared task runner's control. Progress, diagnostics, final outcomes, and cancellation requests belong to that task. The desktop shows active tasks with progress and cancellation controls, and services background work independently of rendering.

The distinction between accepting a command and finishing its work is explicit. An immediate command returns its result. A command that starts background work also returns task IDs, which clients can observe, wait for, or cancel. Success means the required scene changes have been applied; it does not promise that the GPU has already presented a frame.

Scripts and plugin workflows can create child tasks. A parent stays active until its children finish, and cancellation propagates through the task tree. Cancelling stops further work cooperatively; changes already applied to the scene remain. A wait timeout ends the wait without cancelling the underlying task.

Python command methods wait for background work by default, so a script can fetch a structure and then operate on it. Use `wait=False` to get task IDs immediately and manage them through `tasks` or a notebook viewer's `view.tasks`. Browser integrations receive command results and use `viewer.tasks.get`, `list`, `wait`, and `cancel` to follow the same lifecycle.

### Plugins can coordinate work and recover from errors

Plugins now receive typed replies to the host commands they issue, correlated to the original request and delivered only to the requesting plugin. Replies include success or failure, output messages, and every accepted task ID. A command can fail after starting some work, so those IDs remain available for inspection.

For plugin authors, the new contract separates responsibilities clearly:

- **The host owns task identity and state.** A plugin submits a task request and starts its worker after the host accepts it and delivers the assigned ID.
- **Workers report progress, output, and completion.** Scene mutations go through the host and must be acknowledged before the plugin reports success.
- **Nested commands belong to the parent task.** Their completion and cancellation participate in the same lifecycle.
- **Recovery can be deliberate.** Child failure fails the parent by default. An executor can opt into deciding the parent outcome after inspecting and handling a failed child. The original failure remains in task history.

This matters for an agent that needs to correct a command, or a script that has a fallback when a download fails. The parent can handle the error and continue while preserving the record of what happened.

Silent execution also follows the task tree. It suppresses REPL presentation while retaining results, diagnostics, and logs, so a plugin can handle intermediate failures without filling the output panel with messages it intends to recover from.

Polling plugins additionally receive named selections and Recent Atoms paths, including instance qualifiers, alongside object names. They can connect a user's current picks to their own commands without serializing the full scene.

**Plugin migration:** rebuild the application and all native plugins from matching 0.5.0 sources. This release uses **plugin ABI 7** and **runtime wire version 21**. Background workers should use the host task contract, and command callers must inspect returned task IDs before treating a reply as completed work. The [plugin authoring guide](https://github.com/zmactep/patinae/blob/v0.5.0/docs/make-your-own-plugin.md) documents the producer, observation, and cancellation APIs.

### Markdown reaches the output panel

Commands and plugins can now mark individual output messages as Markdown. Desktop and web REPLs render headings, emphasis, lists, and code, making longer answers and small reports easier to read. Desktop tables use a compact monospace layout; unsupported constructs fall back to readable text. Web output is sanitized, and Markdown images are not loaded.

Plain text remains the default. Existing commands keep printing literal text, including punctuation that happens to look like Markdown. Plugins opt in with `print_markdown` or a formatted output message.

The source text and its format survive command replies and plugin polling, including silent execution. **The AI plugin uses Markdown for its final answer**, so explanations and command examples arrive as formatted output in the same panel used for ordinary commands.

### An AI plugin that can operate and inspect the scene

The new native `ai` plugin adds natural-language control through a configurable Responses API endpoint. It can run Patinae commands, wait for the tasks they start, inspect the scene as an image, and use the results to choose its next step.

```text
ai show the loaded structure as a cartoon and color chain A blue
ai what color is the structure now? look at the scene
ai status
ai cancel
```

Command and tool errors are returned to the model so it can attempt a correction within the configured step limit. Scene capture uses the actual renderer and viewport, including when requested from inside a task.

Configure the endpoint, model, and authentication in `ai/config.toml` under the plugin directory. The model must support function calling and image input. API keys can come from an environment variable or directly from configuration; local endpoints can run without authentication. Configuration is reloaded for each request.

Each `ai` request starts a fresh conversation. Command activity is logged, and the REPL displays the final answer or error; this release does not stream answer text or retain conversation history between requests. `ai cancel` stops further work without undoing scene changes already made.

The plugin sends prompts, scene context, command results, requested scene images, and MCP results to the configured model endpoint. Its commands can modify the scene and write files. The [AI plugin guide](https://github.com/zmactep/patinae/blob/v0.5.0/plugins/ai/README.md) covers installation, configuration, and limits.

### MCP tools join the same agent workflow

The AI plugin can connect to configured **Model Context Protocol (MCP)** servers and discover their tools for each request. Both local servers over **stdio** and remote servers over **Streamable HTTP** are supported.

Local server entries specify a command, optional arguments, environment variables, and working directory. Remote entries specify a URL and optional authentication headers, including headers read from environment variables. The discovered tools become available alongside Patinae commands and scene inspection during the agent's run.

Connections close when the request ends, and local server processes are stopped. The current integration exposes MCP tools; resources, prompts, legacy HTTP+SSE, and interactive OAuth login are not supported. External MCP servers are installed and configured separately.

---

## New features

- **Unified background tasks:** shared identity, progress, outcomes, cancellation, and parent/child tracking across desktop, plugins, IPC, Python, and web.
- **Plugin command results:** correlated replies with typed output and accepted task IDs, plus explicit child-error recovery and inherited silent execution.
- **Richer plugin context:** named selections and exact Recent Atoms paths are available during polling.
- **Markdown output:** optional formatting in desktop and web REPLs, preserved through command and plugin transport.
- **AI scene agent:** natural-language commands, task-aware execution, scene image inspection, and formatted final answers.
- **MCP connections:** local stdio and remote Streamable HTTP tools configured per server.

## Bug fixes and improvements

- **Renderer context survives task execution.** Commands issued by tasks retain the native renderer and real viewport size, fixing raster PNG capture from task-owned commands and scripts.
- **Read-only plugin commands avoid session writeback.** Inspecting state no longer causes an unnecessary restore of the serialized session.
- **Selections and scene navigation do less repeated work.** Copy-aware evaluation is shared across selection paths, wildcard projection respects displayed copy membership, and bounds calculations reuse or accumulate intermediate results for `orient`, `center`, and `zoom`.
- **Coloring and structure loading allocate less temporary data.** Full-object coloring uses a contiguous path; residue expansion borrows residue keys; CIF assemblies are collected during parsing; single-model files avoid redundant topology signatures.
- **Plugin and web documentation is expanded.** Reference plugins now have setup and usage guides, and the web README covers embedding, task completion, panels, and deployment requirements.

---

## Downloads

### Bundles

Packages with the application, packaged plugins, and runtime pieces.

| Platform | Architecture | Download |
| --- | --- | --- |
| macOS | Apple Silicon | [Patinae.dmg](https://github.com/zmactep/patinae/releases/download/v0.5.0/Patinae.dmg) |
| Windows | x86_64 | [Windows bundle](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-bundle-windows-x86_64.zip) |
| Windows | ARM64 | [Windows ARM64 bundle](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-bundle-windows-arm64.zip) |

### Standalone executables

| Platform | Architecture | Executable | Plugins |
| --- | --- | --- | --- |
| macOS | Apple Silicon | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-macos-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-plugins-macos-arm64.tar.gz) |
| Windows | x86_64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-windows-x86_64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-plugins-windows-x86_64.zip) |
| Windows | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-windows-arm64.zip) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-plugins-windows-arm64.zip) |
| Linux | x86_64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-linux-x86_64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-plugins-linux-x86_64.tar.gz) |
| Linux | ARM64 | [Executable](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-linux-arm64.tar.gz) | [Plugins](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-plugins-linux-arm64.tar.gz) |

The AI plugin currently requires a separate source build; the release workflow does not yet include it in plugin archives. The local `make plugins` target does include it. Follow the AI plugin guide to install the library and create its configuration.

### Python and web

| Package | Download |
| --- | --- |
| Python wheels | [PyPI package](https://pypi.org/project/patinae/) or release assets |
| Web viewer (WASM + JS) | [patinae-web.tar.gz](https://github.com/zmactep/patinae/releases/download/v0.5.0/patinae-web.tar.gz) |

---

## Release scale

The committed changes since v0.4.7 span **13 commits** and **148 files**, with **21,642 insertions** and **5,551 deletions**, before the 0.5.0 version bump and these notes.

---

**Full Changelog:** [v0.4.7 → v0.5.0](https://github.com/zmactep/patinae/compare/v0.4.7...v0.5.0)
