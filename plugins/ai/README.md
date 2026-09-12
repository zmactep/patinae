# AI plugin

Control Patinae in natural language. The agent can run commands, inspect the scene
as an image, and use tools from configured MCP servers.

```pml
ai show the loaded structure as a cartoon and color chain A blue
ai what color is the structure now? look at the scene
ai status
ai cancel
```

Each `ai` request starts a new conversation. Cancellation stops further work;
already applied changes remain.

## Install and configure

Build from the same source revision as Patinae, then restart the application:

```sh
cargo build --locked -p ai-plugin
mkdir -p ~/.patinae/plugins/ai
cp target/debug/libai_plugin.dylib ~/.patinae/plugins/
cp plugins/ai/config.example.toml ~/.patinae/plugins/ai/config.toml
```

On Linux use `libai_plugin.so`; on Windows use `ai_plugin.dll`.

Run these commands from the repository root. Alternatively, `make plugins`
builds and stages this plugin with the others. See the
[plugin overview](../README.md) for release builds, discovery, and custom
installation directories. In Patinae, `capabilities plugins` and `help ai`
confirm that the command is available.

Edit `config.toml` with your complete Responses API endpoint and a model that
supports function calling and image input:

```toml
endpoint = "https://api.openai.com/v1/responses"
model = "your-model"
api_key = "your-api-key"
max_steps = 24
timeout_seconds = 120
```

Instead of `api_key`, you can use `api_key_env = "OPENAI_API_KEY"` (the default).
A direct key takes precedence. For a local server without authentication, omit
`api_key` and set `api_key_env = ""`.

The config is reloaded for each request. `PATINAE_CONFIG_DIR` changes the base
configuration directory; `PATINAE_PLUGIN_DIR` overrides the plugin directory.
See [config.example.toml](config.example.toml) for all settings.

## MCP servers

Add these sections **after** the top-level settings. Use `command` for a local
stdio server or `url` for a remote Streamable HTTP server:

```toml
[mcp_servers.local]
command = "/path/to/mcp-server"
args = ["--stdio"]
env = { API_KEY = "your-server-key" }

[mcp_servers.remote]
url = "https://mcp.example.com/mcp?option=value"
headers = { Authorization = "Bearer your-server-token" }
```

Install local servers separately. `args`, `env` and `cwd` are optional; the default
working directory is the config directory. Commands run without a shell.
For HTTP headers from environment variables, use
`env_headers = { Authorization = "MCP_AUTHORIZATION" }` with the complete header
value, including `Bearer ` if required. Configure each header in only one map.

Tools are discovered automatically for each request. Connections close when it
ends, and local server processes are stopped. Only MCP tools are exposed;
resources, prompts, legacy HTTP+SSE and interactive OAuth login are not supported.

## Behavior and limits

- Endpoints require HTTPS; HTTP is allowed on loopback. MCP URLs support query
  parameters. URL userinfo, fragments and redirects are not supported.
- Prompts, scene names, command results, requested scene images and MCP results
  go to the model endpoint. MCP calls send arguments to their configured servers.
  The agent can modify the scene and write files.
- Tool errors are returned to the model for correction. Invalid arguments cause
  the entire batch to be skipped. Retries count toward `max_steps` (maximum 128).
- `timeout_seconds` limits model requests and MCP operations (maximum 600).
  Patinae child tasks have no implicit timeout; use `ai cancel`.
- Up to 16 MCP servers and 126 MCP tools are supported. MCP image/audio results
  are returned as JSON, not model image/audio input.
- There is no conversation history between requests or streaming text output.
  Command activity appears in INFO logs; the REPL shows the final answer or error.
  MCP call logs include the server, original tool name and model alias.

## Troubleshooting

- If `ai` is missing, check the plugin directory and startup log, then restart
  Patinae after rebuilding the host and plugin from the same revision.
- If configuration cannot be opened, create `ai/config.toml` under the effective
  plugin directory. The library build does not install the example configuration.
- If authentication fails, check that the configured key environment variable is
  visible to the Patinae process. A key stored directly in the file takes precedence.
- Use the complete Responses endpoint URL, including its path. The model endpoint
  does not accept query parameters; MCP server URLs do.
- For a stalled request, inspect `ai status` and use `ai cancel` before retrying.
  Cancellation does not undo commands already applied to the scene.

## Source

[lib.rs](src/lib.rs) registers the command and manages host tasks;
[worker.rs](src/worker.rs) runs the agent loop; [config.rs](src/config.rs)
validates configuration; [mcp.rs](src/mcp.rs) connects MCP tools; and
[screenshot.rs](src/screenshot.rs) prepares scene images.
