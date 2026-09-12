//! Request-scoped MCP clients and model tool routing on the AI worker.

use crate::config::McpServerConfig;
use patinae_plugin::tasks::TaskError;
use rmcp::{
    model::{
        CallToolRequest, CallToolRequestParams, CancelledNotificationParam, ClientRequest,
        PaginatedRequestParams, RequestId, ServerResult, Tool,
    },
    service::{PeerRequestOptions, RunningService},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    RoleClient, ServiceExt,
};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
    process::Stdio,
    time::Duration,
};
use tokio::{
    process::{Child, Command},
    time::timeout,
};

// Responses accepts at most 128 tools; reserve two slots for Patinae tools.
const MAX_MCP_TOOLS: usize = 126;
// Bound discovery even when a server returns empty pages with fresh cursors.
const MAX_TOOL_PAGES: usize = 32;
const MAX_TOOL_BYTES: usize = 1024 * 1024;
// Cleanup must also finish after cancellation or a broken remote connection.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

struct Connection {
    name: String,
    client: RunningService<RoleClient, ()>,
}

struct RemoteTool {
    connection: usize,
    name: String,
    definition: Value,
}

#[derive(Default)]
pub(crate) struct McpConnections {
    connections: Vec<Connection>,
    children: Vec<Child>,
    tools: BTreeMap<String, RemoteTool>,
    active_call: Option<(usize, RequestId)>,
}

impl McpConnections {
    pub async fn connect(
        &mut self,
        servers: &BTreeMap<String, McpServerConfig>,
        config_dir: &Path,
        request_timeout: Duration,
    ) -> Result<(), TaskError> {
        for (name, config) in servers {
            let connect = async {
                if let Some(command) = &config.command {
                    let mut command = Command::new(command);
                    command
                        .args(&config.args)
                        .envs(&config.env)
                        .current_dir(
                            config
                                .cwd
                                .as_ref()
                                .map_or_else(|| config_dir.to_owned(), |cwd| config_dir.join(cwd)),
                        )
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .kill_on_drop(true);
                    let mut child = command
                        .spawn()
                        .map_err(|_| failure(name, "cannot start stdio server"))?;
                    let stdout = child.stdout.take().expect("piped stdout");
                    let stdin = child.stdin.take().expect("piped stdin");
                    // Retain the process before awaiting initialization so cancellation
                    // and failed handshakes still kill and reap it during shutdown.
                    self.children.push(child);
                    ().serve((stdout, stdin))
                        .await
                        .map_err(|_| failure(name, "initialization failed"))
                } else {
                    let url = config
                        .url
                        .as_ref()
                        .ok_or_else(|| failure(name, "missing URL"))?;
                    let client = reqwest::Client::builder()
                        .connect_timeout(request_timeout)
                        .redirect(reqwest::redirect::Policy::none())
                        .build()
                        .map_err(|_| failure(name, "cannot create HTTP client"))?;
                    let transport = StreamableHttpClientTransport::with_client(
                        client,
                        StreamableHttpClientTransportConfig::with_uri(url.clone())
                            .custom_headers(config.http_headers()?)
                            .max_concurrent_requests(1)
                            .max_sse_event_size(MAX_TOOL_BYTES)
                            .control_request_timeout(SHUTDOWN_TIMEOUT),
                    );
                    ().serve(transport)
                        .await
                        .map_err(|_| failure(name, "initialization failed"))
                }
            };
            let client = timeout(request_timeout, connect)
                .await
                .map_err(|_| failure(name, "initialization timed out"))??;
            let index = self.connections.len();
            self.connections.push(Connection {
                name: name.clone(),
                client,
            });
            timeout(request_timeout, self.discover(index))
                .await
                .map_err(|_| failure(name, "tool discovery timed out"))??;
        }
        Ok(())
    }

    async fn discover(&mut self, index: usize) -> Result<(), TaskError> {
        let connection = &self.connections[index];
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut names = HashSet::new();
        for _ in 0..MAX_TOOL_PAGES {
            let page = connection
                .client
                .list_tools(
                    cursor
                        .map(|cursor| PaginatedRequestParams::default().with_cursor(Some(cursor))),
                )
                .await
                .map_err(|_| failure(&connection.name, "tool discovery failed"))?;
            for tool in page.tools {
                if !names.insert(tool.name.to_string()) || self.tools.len() >= MAX_MCP_TOOLS {
                    return Err(failure(
                        &connection.name,
                        "duplicate tools or too many tools (maximum 126 total)",
                    ));
                }
                let alias = format!("mcp_{}_{}", connection.name, names.len());
                let definition = tool_definition(&alias, &connection.name, &tool)?;
                self.tools.insert(
                    alias,
                    RemoteTool {
                        connection: index,
                        name: tool.name.into_owned(),
                        definition,
                    },
                );
            }
            cursor = page.next_cursor;
            match &cursor {
                None => return Ok(()),
                Some(cursor) if cursors.insert(cursor.clone()) => {}
                _ => return Err(failure(&connection.name, "repeated tool-list cursor")),
            }
        }
        Err(failure(&connection.name, "too many tool-list pages"))
    }

    pub fn definitions(&self) -> impl Iterator<Item = Value> + '_ {
        self.tools.values().map(|tool| tool.definition.clone())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn call_log(&self, alias: &str) -> Result<String, TaskError> {
        let tool = self
            .tools
            .get(alias)
            .ok_or_else(|| TaskError::new("mcp", "unknown MCP tool"))?;
        // Quote remote names so embedded newlines cannot create extra log entries.
        Ok(format!(
            "AI MCP tool: server={:?}, tool={:?}, alias={alias}",
            self.connections[tool.connection].name, tool.name
        ))
    }

    pub async fn call(
        &mut self,
        alias: &str,
        arguments: Map<String, Value>,
        request_timeout: Duration,
    ) -> Result<Value, TaskError> {
        let tool = self
            .tools
            .get(alias)
            .ok_or_else(|| TaskError::new("mcp", "unknown MCP tool"))?;
        let connection = &self.connections[tool.connection];
        let params = CallToolRequestParams::new(tool.name.clone()).with_arguments(arguments);
        let handle = timeout(
            request_timeout,
            connection.client.send_request_with_option(
                ClientRequest::CallToolRequest(CallToolRequest::new(params)),
                PeerRequestOptions::with_timeout(request_timeout),
            ),
        )
        .await
        .map_err(|_| failure(&connection.name, "tool dispatch timed out"))?
        .map_err(|_| failure(&connection.name, "tool dispatch failed"))?;
        self.active_call = Some((tool.connection, handle.id.clone()));
        // The SDK sends notifications/cancelled on timeout. Keep the request ID
        // outside this future so host cancellation can send the same notification.
        let result = timeout(request_timeout + SHUTDOWN_TIMEOUT, handle.await_response())
            .await
            .map_err(|_| {
                failure(
                    &connection.name,
                    "tool call timed out; remote effects may have occurred",
                )
            })?;
        self.active_call = None;
        let result = result.map_err(|_| {
            failure(
                &connection.name,
                "tool call failed or timed out; remote effects may have occurred",
            )
        })?;
        let ServerResult::CallToolResult(result) = result else {
            return Err(failure(&connection.name, "unexpected tool result"));
        };
        let value = json!({"ok": !result.is_error.unwrap_or(false), "server": connection.name,
            "tool": tool.name, "result": result});
        if value.to_string().len() > MAX_TOOL_BYTES {
            return Err(failure(&connection.name, "tool result exceeded 1 MiB"));
        }
        Ok(value)
    }

    pub async fn close(&mut self) {
        if let Some((index, id)) = self.active_call.take() {
            let _ = timeout(
                SHUTDOWN_TIMEOUT,
                self.connections[index]
                    .client
                    .notify_cancelled(CancelledNotificationParam::new(
                        Some(id),
                        Some("AI request cancelled".into()),
                    )),
            )
            .await;
        }
        // Stop local processes before awaiting transport cleanup. kill_on_drop is
        // a final fallback; wait() reaps each direct child while the runtime lives.
        for child in &mut self.children {
            let _ = child.start_kill();
        }
        for connection in &mut self.connections {
            connection.client.cancellation_token().cancel();
        }
        let _ = timeout(SHUTDOWN_TIMEOUT, async {
            for connection in &mut self.connections {
                let _ = connection.client.close().await;
            }
        })
        .await;
        let _ = timeout(SHUTDOWN_TIMEOUT, async {
            for child in &mut self.children {
                let _ = child.wait().await;
            }
        })
        .await;
        self.tools.clear();
        self.connections.clear();
        self.children.clear();
    }
}

fn tool_definition(alias: &str, server: &str, tool: &Tool) -> Result<Value, TaskError> {
    if tool.name.trim().is_empty()
        || tool.input_schema.get("type").and_then(Value::as_str) != Some("object")
    {
        return Err(failure(
            server,
            "tool requires a name and object input schema",
        ));
    }
    // MCP schemas may have optional properties; strict Responses schemas would
    // change their meaning. Route aliases without modifying the original schema.
    let value = json!({"type": "function", "strict": false, "name": alias,
        "description": format!("MCP server {server}, tool {}: {}", tool.name, tool.description.as_deref().unwrap_or_default()),
        "parameters": tool.input_schema});
    if value.to_string().len() > MAX_TOOL_BYTES {
        return Err(failure(server, "tool definition exceeded 1 MiB"));
    }
    Ok(value)
}

fn failure(server: &str, message: &str) -> TaskError {
    // SDK errors can contain URLs, headers or remote output. Only expose our
    // fixed message and the validated configuration name to the host/model.
    TaskError::new("mcp", format!("MCP server {server}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn fixture() -> (
        McpConnections,
        tokio::sync::mpsc::UnboundedReceiver<String>,
        tokio::task::JoinHandle<()>,
    ) {
        let (client, server) = tokio::io::duplex(8192);
        let (events, received) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(server);
            let mut lines = BufReader::new(read).lines();
            while let Some(line) = lines.next_line().await.unwrap() {
                let request: Value = serde_json::from_str(&line).unwrap();
                let method = request["method"].as_str().unwrap();
                let _ = events.send(method.to_owned());
                let result = match method {
                    "initialize" => json!({"protocolVersion": request["params"]["protocolVersion"],
                        "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"}}),
                    "tools/list" => {
                        let second = request["params"]["cursor"] == "page2";
                        let mut page = json!({"tools": [{"name": if second { "other/tool" } else { "command" },
                            "inputSchema": {"type": "object", "properties": {"fail": {"type": "boolean"}}}}]});
                        if !second {
                            page["nextCursor"] = json!("page2");
                        }
                        page
                    }
                    "tools/call" => {
                        if request["params"]["arguments"]["wait"] == true {
                            continue;
                        }
                        json!({"content": [{"type": "text", "text": "fixture result"}],
                            "structuredContent": {"name": request["params"]["name"]},
                            "isError": request["params"]["arguments"]["fail"] == true})
                    }
                    _ if request.get("id").is_none() => continue,
                    _ => panic!("unexpected method: {method}"),
                };
                let response = json!({"jsonrpc": "2.0", "id": request["id"], "result": result});
                if write
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let client = ().serve(client).await.unwrap();
        let mut mcp = McpConnections::default();
        mcp.connections.push(Connection {
            name: "fixture".into(),
            client,
        });
        mcp.discover(0).await.unwrap();
        (mcp, received, task)
    }

    #[tokio::test]
    async fn discovery_pages_namespaces_and_tool_errors_round_trip() {
        let (mut mcp, _, server) = fixture().await;
        let tools: Vec<_> = mcp.definitions().collect();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "mcp_fixture_1");
        assert_eq!(tools[0]["strict"], false);
        assert!(tools[0]["parameters"].get("required").is_none());
        assert!(!mcp.contains("command"));
        let result = mcp
            .call("mcp_fixture_2", Map::new(), Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(result["result"]["structuredContent"]["name"], "other/tool");
        assert_eq!(result["ok"], true);
        let result = mcp
            .call(
                "mcp_fixture_1",
                json!({"fail": true}).as_object().unwrap().clone(),
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["result"]["content"][0]["text"], "fixture result");
        assert!(mcp
            .call("not_advertised", Map::new(), Duration::from_secs(1))
            .await
            .is_err());
        mcp.close().await;
        timeout(Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn timeout_and_dropped_calls_notify_cancellation_and_close() {
        for host_cancel in [false, true] {
            let (mut mcp, mut events, server) = fixture().await;
            let call = mcp.call(
                "mcp_fixture_1",
                json!({"wait": true}).as_object().unwrap().clone(),
                Duration::from_millis(100),
            );
            if host_cancel {
                tokio::pin!(call);
                tokio::select! {
                    result = &mut call => panic!("unexpected result: {result:?}"),
                    _ = async { while events.recv().await.as_deref() != Some("tools/call") {} } => {}
                }
            } else {
                assert!(call.await.is_err());
            }
            mcp.close().await;
            timeout(Duration::from_secs(1), async {
                loop {
                    match events.recv().await.as_deref() {
                        Some("notifications/cancelled") => break,
                        Some(_) => {}
                        None => panic!("missing cancellation notification"),
                    }
                }
            })
            .await
            .unwrap();
            timeout(Duration::from_secs(1), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdio_uses_args_env_cwd_and_reaps_process_after_success_or_failed_startup() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("marker"), "fixture").unwrap();
        let script = r#"
test "$1" = 'fixture-arg' && test "$MCP_FIXTURE" = 'fixture-env' && test -f marker || exit 1
while IFS= read -r line; do
    id=$(printf '%s' "$line" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
    case "$line" in
        *'"method":"initialize"'*) result='{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"stdio-fixture","version":"1"}}' ;;
        *'"method":"tools/list"'*) result='{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}' ;;
        *'"method":"tools/call"'*) result='{"content":[{"type":"text","text":"stdio result"}]}' ;;
        *) continue ;;
    esac
    printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
done
"#;
        for success in [true, false] {
            let config: McpServerConfig = serde_json::from_value(
                json!({"command": "/bin/sh", "args": ["-c", script, "fixture", "fixture-arg"],
                "env": {"MCP_FIXTURE": if success { "fixture-env" } else { "wrong" }}, "cwd": "."}),
            )
            .unwrap();
            let mut mcp = McpConnections::default();
            let result = mcp
                .connect(
                    &BTreeMap::from([("local".into(), config)]),
                    root.path(),
                    Duration::from_secs(2),
                )
                .await;
            assert_eq!(result.is_ok(), success);
            if success {
                let result = mcp
                    .call("mcp_local_1", Map::new(), Duration::from_secs(1))
                    .await
                    .unwrap();
                assert_eq!(result["result"]["content"][0]["text"], "stdio result");
            }
            let pid = mcp.children[0].id().unwrap();
            mcp.close().await;
            assert!(!Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .await
                .unwrap()
                .success());
        }
    }
}
