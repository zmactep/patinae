# AI plugin

`ai <prompt>` runs one natural-language request through the Responses API
with function calling. The model discovers commands
from the running host's `help` and `capabilities`, executes commands sequentially,
and receives their output after accepted child tasks finish. The `capture_scene`
tool lets the model see the current scene as an image.

```pml
ai show the loaded structure as a cartoon, color chain A blue and save /tmp/structure.png
ai status
ai cancel
ai what color is the structure now? look at the scene
```

The entire remainder of the `ai` line is the prompt, including commas,
semicolons and `#`. Exact `status` and `cancel` are reserved control requests.
Cancellation is cooperative and preserves changes already applied.

## Build and configure

From the repository root:

```sh
cargo build --locked -p ai-plugin
mkdir -p ~/.patinae/plugins/ai
cp target/debug/libai_plugin.dylib ~/.patinae/plugins/
cp plugins/ai/config.example.toml ~/.patinae/plugins/ai/config.toml
```

On Linux use `libai_plugin.so`; on Windows use `ai_plugin.dll`. The plugin and
Patinae must be built from matching SDK versions. Restart Patinae after installing.
`make plugins` also includes this plugin in the normal release staging directory.

Edit `config.toml` to set the **complete** Responses endpoint URL (normally
`https://api.openai.com/v1/responses`) and a model that supports function tool
calls and image input. For a hosted endpoint, use HTTPS and set the key directly:

```toml
api_key = "your-api-key"
```

Alternatively, set `api_key_env` to the name of the environment variable containing
the key (defaults to `OPENAI_API_KEY`). That variable must be in the Patinae process
environment. When present, `api_key` takes precedence over `api_key_env`; an empty
or whitespace-only key is rejected. For a local server without authentication,
omit `api_key` and set `api_key_env = ""`.

The default configuration path is `~/.patinae/plugins/ai/config.toml`.
`PATINAE_CONFIG_DIR` changes the base configuration directory;
`PATINAE_PLUGIN_DIR` overrides the plugin directory. The file is reloaded for
each request. A missing file or invalid configuration produces an actionable
error; ordinary Patinae commands remain available.

Starting an AI request sends the prompt, loaded object/selection/recent-atom
names, live help/capabilities, subsequent command outputs, and scene images when
the model calls `capture_scene` to the configured
endpoint. Model commands run with normal Patinae capabilities and can change the
scene and write files. Use an endpoint and model appropriate for your data.

## Execution and limits

- The host owns TaskIds, scene validity, cancellation, child relationships and
  terminal outcomes. There is no separate AI task registry.
- The `ai` command requests no full-session snapshot. Large structures stay in
  the host; only object/selection/recent-atom names are read during polling.
- A worker thread runs an async HTTP client and conversation. Polling only moves
  requests/replies through channels. Unloading stops and joins the worker.
- Each command sent to the host is logged at INFO with its task and request IDs.
  The complete command is quoted with escaped newlines, so multiline Python
  remains identifiable in a single log entry, including when execution fails.
- AI work is silent in the REPL, including child Python/PML stdout, warnings,
  recoverable errors and timing. Captured receipts and child diagnostics still
  reach the model. The final AI answer or terminal error is displayed normally.
- INFO task-progress logs show each `capture_scene` source/output size and base64
  byte count, each Responses request's model, image count and body size, and the
  HTTP status. Image contents, request bodies and credentials are not logged.
- `capture_scene` takes no arguments. It executes the host's `png` command at
  the current viewport size, reads the result on the worker, and sends a PNG
  data URL as an `input_image` observation following the correlated tool outputs.
  Capture uses the current camera and displayed CPU/GPU image when one exists.
  Normal raster captures include the scene but omit application panels and
  interactive selection/hover markers, as the standard PNG exporter does.
- Images are proportionally reduced to at most 1280 pixels on the longest edge.
  Only the latest image remains in conversation history; older image receipts
  remain as text. The private temporary directory is removed after capture,
  including on error or cancellation. Use ordinary `png` for persistent exports.
  The prompt directs the model to capture as its first tool for visual questions,
  then answer once sufficient information is available. It distinguishes exact
  material values from approximate observed RGB/hex and discourages Python API
  introspection and image-decoder implementation for ordinary visual questions.
- Each request uses `input`, `instructions`, flat function tool definitions and
  `store = false`. The plugin retains output items, including opaque reasoning
  with `reasoning.encrypted_content`, and returns command results as
  `function_call_output` items correlated by `call_id`. Failed, incomplete or
  malformed responses stop execution before any tool in that response runs.
- Command failures return `ok: false`, a structured `error`, the captured receipt
  and completed child outcomes to the model for correction. All accepted children
  are awaited, including after a failed dispatch. Remaining calls in the same
  batch receive `skipped_after_error` without execution. Corrections consume the
  normal `max_steps` budget; applied changes are not rolled back.
- The AI task opts into `ChildFailurePolicy::ParentDecides`: a failed attempt
  remains in host history with its diagnostics and effects, but does not force
  a corrected request to fail. The model must inspect partial effects before
  retrying and explain any unresolved error. Cancellation, stale context, missing
  executors and host/protocol failures still stop the request.
- A replaced session rejects subsequent commands and the final answer with the
  host's `stale_context` error. Replacement alone does not interrupt pending HTTP;
  `ai cancel` does. The final answer also requires a host-acknowledged no-op.
- `max_steps` bounds model turns (default 24, maximum 128); a response may contain
  at most 8 sequential tool calls. `timeout_seconds` bounds each HTTP request
  (default 120, maximum 600). Child tasks have no implicit timeout; use `ai cancel`.
- Replies are capped at 1 MiB; outgoing conversation JSON at 16 MiB; prompt,
  individual command and retained answer at 16 KiB. Long answers are marked as
  truncated. Redirects are disabled; endpoint error bodies and credentials are
  not printed.
- Captured files are capped at 32 MiB, source dimensions at 8192 per edge,
  decoder allocation at 256 MiB, and prepared PNGs at 8 MiB. Image export/read/
  decode failures return structured errors for correction.

This first version has no chat history between invocations, streaming text or
other API transports. The initial scene
context lists names; further inspection uses commands discovered from host help.

## Validation

```sh
cargo build --locked --offline -p ai-plugin
cargo test --locked --offline -p ai-plugin --lib
PATINAE_AI_TEST_LIBRARY="$PWD/target/debug/libai_plugin.dylib" \
  cargo test --locked --offline -p ai-plugin --lib -- --ignored
cargo clippy --locked --offline -p ai-plugin --all-targets -- -D warnings
```

The opt-in ABI smoke test loads the built AI library through PluginHost and
uses a local Responses server. It checks command result correlation, opaque
reasoning replay, error feedback and correction, silent output, image input and
cancellation. The image fixture is a two-pixel viewport image whose PNG bytes
are decoded from the outgoing request. No Python plugin, molecular files, GPU,
credentials or live model are needed. This test checks transport and host
integration; it does not validate rendering or model behavior.

The transport follows the [Responses function-calling contract](https://developers.openai.com/api/docs/guides/function-calling).
Image observations follow the [Responses image-input format](https://developers.openai.com/api/docs/guides/images-vision).
