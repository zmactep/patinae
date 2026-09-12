# IPC plugin

Control a running Patinae application through a local Unix domain socket.
Clients can execute commands, inspect scene state, observe background tasks,
and register commands whose implementations run in the client process.

The server supports macOS and Linux. Windows socket serving is not implemented.
The plugin has no REPL commands or GUI panel.

## Build and start

From the repository root, using the same revision as the Patinae application:

```sh
cargo build --locked --release -p ipc-plugin
mkdir -p ~/.patinae/plugins
cp target/release/libipc_plugin.dylib ~/.patinae/plugins/
PATINAE_IPC_SOCKET=/tmp/patinae-control.sock ./target/release/patinae
```

On Linux copy `libipc_plugin.so`. Build the application first with
`make patinae` if needed. See the [plugin overview](../README.md) for discovery
and alternative installation directories.

Set `PATINAE_IPC_SOCKET` before starting Patinae. Without it, the plugin loads
but leaves the server disabled. Use a short socket path with an existing parent
directory and a separate path for each application instance. A bind failure is
reported in the startup log.

## Protocol quick start

Messages are UTF-8 JSON objects, one per line, with a case-sensitive `type`
field. Keep the connection open while receiving replies. The server accepts
one client at a time.

Send this handshake first:

```json
{"type":"Hello","client_id":"my-client","protocol_version":2}
```

The successful reply is:

```json
{"type":"Ok","id":0}
```

Then send requests with distinct numeric IDs and match responses by `id`:

```json
{"type":"Ping","id":1}
{"type":"GetNames","id":2}
{"type":"CountAtoms","id":3,"selection":"all"}
{"type":"Execute","id":4,"command":"color cyan, all","silent":false}
```

`Ping` returns `Pong`; scene queries return `Value`; `Execute` returns
`Execution`. Requests can complete asynchronously. A new connection must perform
its own handshake. Only `Ping` and `Capabilities` are available before it.

## Commands and tasks

An `Execution` reply contains `result`, typed `messages`, and `task_ids` alongside
the request ID. Rust `Result` values use JSON objects such as `{"Ok":null}` or
`{"Err":"message"}`. `silent` suppresses presentation without discarding the
receipt or task outcome.

A successful command is complete when `task_ids` is empty. Otherwise retain
the returned opaque task IDs and use these requests:

| Request | Purpose |
| --- | --- |
| `Capabilities` | Discover protocol version, tracked task kinds, and retention limits |
| `GetTask` | Read one task's state and retained outcome |
| `ListTasks` | Read a bounded page of task summaries |
| `WaitTask` | Wait for completion, optionally with `timeout_ms` |
| `CancelTask` | Request cooperative cancellation |

`GetTask`, `WaitTask`, and `CancelTask` require `id` and `task_id`. `ListTasks`
accepts an optional `request` object. Task IDs are strings and must be passed
back unchanged. Cancellation acknowledgement is not completion; wait for the
terminal state. A wait timeout does not cancel the task. Retained results can
expire, so use the limits returned by `Capabilities`.

A failed command may still have accepted tasks. Observe those IDs rather than
retrying the whole command. Disconnecting a client does not automatically cancel
ordinary host tasks; reconnect and query their IDs.

## Other operations

`GetState` returns scene information, `GetView` returns the 18-value view,
`ShowWindow` and `HideWindow` control visibility, and `Quit` closes the application.

`RegisterCommand` and `UnregisterCommand` manage client-provided commands.
An invocation produces a `CallbackRequest` carrying `id`, `task_id`, `name`, and
`args`. The client returns `CallbackResponse` with those IDs and a task outcome,
and handles `CallbackCancel` for cooperative cancellation. Commands issued from
a callback can carry its task ID as `parent_id`.

See [protocol.rs](src/protocol.rs) for the complete request/response schema and
[handler.rs](src/handler.rs) for routing. The [Python package](../../python/README.md)
contains the external scripting interface that uses this integration.

## Limits and troubleshooting

- The server is local and has no authentication handshake; filesystem access
  to the socket controls who can issue application commands.
- JSON frames are limited to 256 KiB. Oversized or invalid input is rejected.
- A second client is rejected while the current connection or its pending
  replies occupy the client slot.
- `protocol_required` means the handshake is missing; `unsupported_protocol`
  means the client and server protocol versions differ.
- Being listed by `capabilities plugins` confirms library loading, but does
  not confirm a successful socket bind. Check the startup log and socket path.
