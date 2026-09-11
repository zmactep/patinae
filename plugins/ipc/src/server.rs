//! IPC Server implementation
//!
//! Provides a non-blocking IPC server using Unix domain sockets.
//! Uses no tokio dependencies or callback channels; async handling goes
//! through the plugin's `PollContext`.
//!
//! Closing the request stream preserves complete requests and their pending replies.
//! Once replies drain, the server closes that connection. Pending and buffered replies
//! reserve the single-client slot even after request EOF. A failed reply write detects
//! a fully disconnected peer; running host tasks remain available through lookup.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::protocol::{IpcRequest, IpcResponse};

// A snapshot may occupy 64 KiB before its JSON response envelope is added.
const MAX_FRAME_BYTES: usize = 256 * 1024;
const MAX_BUFFER_BYTES: usize = 1024 * 1024;
const IO_BYTES_PER_TICK: usize = 256 * 1024;
const OUTPUT_FRAMES_PER_TICK: usize = 64;

/// IPC Server for external control
///
/// Listens on a Unix domain socket and processes incoming requests.
/// The server is non-blocking and should be polled each frame.
pub struct IpcServer {
    /// Path to the socket file
    socket_path: PathBuf,
    /// Listener for incoming connections
    #[cfg(unix)]
    listener: std::os::unix::net::UnixListener,
    /// Currently connected client stream
    #[cfg(unix)]
    client: Option<ClientConnection>,
    /// Next callback ID counter
    next_callback_id: AtomicU64,
    generation: u64,
    pending_replies: usize,
}

#[cfg(unix)]
struct ClientConnection {
    stream: std::os::unix::net::UnixStream,
    input: Vec<u8>,
    output: VecDeque<Vec<u8>>,
    output_offset: usize,
    output_bytes: usize,
    client_id: String,
    read_eof: bool,
}

impl IpcServer {
    /// Create and bind a new IPC server
    #[cfg(unix)]
    pub fn bind(socket_path: &Path) -> std::io::Result<Self> {
        use std::os::unix::net::UnixListener;

        prepare_socket_path(socket_path, &ProcessSocketCleanup)?;

        let listener = UnixListener::bind(socket_path)?;
        listener.set_nonblocking(true)?;

        log::info!("IPC server listening on {:?}", socket_path);

        Ok(Self {
            socket_path: socket_path.to_path_buf(),
            listener,
            client: None,
            next_callback_id: AtomicU64::new(1),
            generation: 0,
            pending_replies: 0,
        })
    }

    #[cfg(not(unix))]
    pub fn bind(socket_path: &Path) -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "IPC server not yet supported on this platform",
        ))
    }

    /// Generate the next callback ID
    pub fn next_callback_id(&self) -> u64 {
        self.next_callback_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Advance bounded socket I/O before routing responses for this host tick.
    #[cfg(unix)]
    pub fn begin_tick(&mut self) {
        // Observe EOF before accepting replacements or delivering pending replies.
        self.read_available();
        // Drain complete requests before considering another connection.
        if self
            .client
            .as_ref()
            .is_some_and(|client| client.read_eof && client.input.contains(&b'\n'))
        {
            return;
        }
        match self.listener.accept() {
            Ok((stream, _addr)) => {
                if self.client.as_ref().is_some_and(|client| {
                    !client.read_eof || self.pending_replies > 0 || !client.output.is_empty()
                }) {
                    log::warn!("IPC: rejecting second client, only one connection allowed");
                    Self::reject_connection(stream);
                } else {
                    log::info!("IPC client connected");
                    if let Err(e) = stream.set_nonblocking(true) {
                        log::error!("Failed to set non-blocking: {}", e);
                        return;
                    }
                    let Some(generation) = self.generation.checked_add(1) else {
                        return;
                    };
                    self.generation = generation;
                    self.pending_replies = 0;
                    self.client = Some(ClientConnection {
                        stream,
                        input: Vec::new(),
                        output: VecDeque::new(),
                        output_offset: 0,
                        output_bytes: 0,
                        client_id: String::from("unknown"),
                        read_eof: false,
                    });
                    self.read_available();
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => {
                log::error!("Failed to accept connection: {}", e);
            }
        }
    }

    #[cfg(not(unix))]
    pub fn begin_tick(&mut self) {}

    #[cfg(unix)]
    fn read_available(&mut self) {
        let Some(client) = self.client.as_mut() else {
            return;
        };
        if client.read_eof {
            return;
        }
        let mut buffer = [0_u8; 8192];
        let mut remaining = IO_BYTES_PER_TICK;
        while remaining > 0 {
            let count = buffer.len().min(remaining);
            match client.stream.read(&mut buffer[..count]) {
                Ok(0) => {
                    // A write half-close still permits requests to receive responses.
                    client.read_eof = true;
                    return;
                }
                Ok(count) => {
                    remaining -= count;
                    client.input.extend_from_slice(&buffer[..count]);
                    if client.input.len() > MAX_BUFFER_BYTES
                        || client
                            .input
                            .rsplit(|byte| *byte == b'\n')
                            .next()
                            .is_some_and(|tail| tail.len() > MAX_FRAME_BYTES)
                    {
                        log::warn!("IPC input buffer limit exceeded");
                        self.client = None;
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return,
                Err(error) => {
                    log::debug!("IPC read failed: {error}");
                    self.client = None;
                    return;
                }
            }
        }
    }

    /// Read one complete request, retaining partial input across host ticks.
    #[cfg(unix)]
    pub fn poll(&mut self) -> Option<IpcRequest> {
        let client = self.client.as_mut()?;
        let Some(end) = client.input.iter().position(|byte| *byte == b'\n') else {
            if client.read_eof {
                client.input.clear();
            }
            return None;
        };
        if end > MAX_FRAME_BYTES {
            self.client = None;
            return None;
        }
        let decoded = serde_json::from_slice(&client.input[..end]);
        // Schema errors must still resolve a well-formed client's correlation ID.
        let invalid_response = decoded
            .as_ref()
            .err()
            .and_then(|error| invalid_request_response(&client.input[..end], &error.to_string()));
        client.input.drain(..=end);
        match decoded {
            Ok(request) => Some(request),
            Err(error) => {
                log::warn!("Invalid IPC request: {error}");
                if let Some(response) = invalid_response {
                    if let Err(error) = self.send(response) {
                        log::debug!("IPC invalid request reply failed: {error}");
                    }
                }
                None
            }
        }
    }

    #[cfg(not(unix))]
    pub fn poll(&mut self) -> Option<IpcRequest> {
        None
    }

    /// Queue a bounded response without blocking on a slow client.
    ///
    /// # Errors
    /// Returns an error for disconnected clients, invalid encoding, or buffer overflow.
    #[cfg(unix)]
    pub fn send(&mut self, response: IpcResponse) -> std::io::Result<()> {
        if let Some(ref mut client) = self.client {
            let mut bytes = serde_json::to_vec(&response)?;
            bytes.push(b'\n');
            if bytes.len() > MAX_FRAME_BYTES || client.output_bytes + bytes.len() > MAX_BUFFER_BYTES
            {
                self.client = None;
                return Err(std::io::Error::other("IPC response buffer limit exceeded"));
            }
            client.output_bytes += bytes.len();
            client.output.push_back(bytes);
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No client connected",
            ))
        }
    }

    /// Flush a bounded amount, retaining unwritten bytes on backpressure.
    #[cfg(unix)]
    pub fn end_tick(&mut self) {
        if let Some(client) = self.client.as_mut() {
            if let Err(error) = flush_output(
                &mut client.stream,
                &mut client.output,
                &mut client.output_offset,
                &mut client.output_bytes,
                IO_BYTES_PER_TICK,
            ) {
                log::debug!("IPC write failed: {error}");
                self.client = None;
            }
        }
        if self.client.as_ref().is_some_and(|client| {
            client.read_eof
                && client.input.is_empty()
                && client.output.is_empty()
                && self.pending_replies == 0
        }) {
            self.client = None;
        }
    }

    /// Keep a write-half-closed connection until its accepted requests resolve.
    pub fn set_pending_replies(&mut self, count: usize) {
        self.pending_replies = count;
    }

    #[cfg(not(unix))]
    pub fn end_tick(&mut self) {}

    /// Current connection identity, independent of client-provided request IDs.
    pub fn connection_generation(&self) -> Option<u64> {
        #[cfg(unix)]
        {
            self.client.as_ref().map(|_| self.generation)
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    #[cfg(not(unix))]
    pub fn send(&mut self, _response: IpcResponse) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "IPC server not yet supported on this platform",
        ))
    }

    /// Set the connected client's identifier (called when Hello is received)
    pub fn set_client_id(&mut self, id: String) {
        #[cfg(unix)]
        if let Some(ref mut client) = self.client {
            client.client_id = id;
        }
    }

    /// Reject an incoming connection by sending an error and closing
    #[cfg(unix)]
    fn reject_connection(mut stream: std::os::unix::net::UnixStream) {
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        let response = IpcResponse::Error {
            id: 0,
            message: "Another client is already connected".to_string(),
        };
        if let Ok(json) = serde_json::to_string(&response) {
            let _ = writeln!(stream, "{}", json);
            let _ = stream.flush();
        }
    }
}

#[cfg(any(unix, test))]
fn invalid_request_response(bytes: &[u8], error: &str) -> Option<IpcResponse> {
    use patinae_plugin::tasks::{TaskId, TaskLookupError};
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let id = value.get("id")?.as_u64()?;
    let invalid_task_id = value
        .get("task_id")
        .and_then(serde_json::Value::as_str)
        .is_none_or(|id| id.parse::<TaskId>().is_err());
    match value.get("type").and_then(serde_json::Value::as_str) {
        Some("GetTask") if invalid_task_id => Some(IpcResponse::Task {
            id,
            result: Err(TaskLookupError::InvalidId),
        }),
        Some("CancelTask") if invalid_task_id => Some(IpcResponse::TaskCancellation {
            id,
            result: Err(TaskLookupError::InvalidId),
        }),
        _ => Some(IpcResponse::Error {
            id,
            message: format!("Invalid IPC request: {error}"),
        }),
    }
}

#[cfg(any(unix, test))]
fn flush_output(
    writer: &mut impl Write,
    frames: &mut VecDeque<Vec<u8>>,
    offset: &mut usize,
    queued_bytes: &mut usize,
    mut budget: usize,
) -> std::io::Result<()> {
    let mut frames_left = OUTPUT_FRAMES_PER_TICK;
    while budget > 0 && frames_left > 0 {
        let Some(frame) = frames.front() else {
            break;
        };
        let end = frame.len().min(*offset + budget);
        match writer.write(&frame[*offset..end]) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(count) => {
                *offset += count;
                *queued_bytes -= count;
                budget -= count;
                if *offset == frame.len() {
                    frames.pop_front();
                    *offset = 0;
                    frames_left -= 1;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => break,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(any(unix, test))]
trait SocketCleanup {
    fn remove_file(&self, path: &Path) -> std::io::Result<()>;
}

#[cfg(unix)]
struct ProcessSocketCleanup;

#[cfg(unix)]
impl SocketCleanup for ProcessSocketCleanup {
    fn remove_file(&self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }
}

#[cfg(any(unix, test))]
fn prepare_socket_path(path: &Path, cleanup: &impl SocketCleanup) -> std::io::Result<()> {
    match cleanup.remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
        log::info!("IPC server stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[cfg(unix)]
    #[test]
    fn fragmented_utf8_requests_survive_multiple_ticks() {
        use std::os::unix::net::UnixStream;
        let path = crate::tests::socket_path("fragment");
        let mut server = IpcServer::bind(&path).unwrap();
        let mut client = UnixStream::connect(&path).unwrap();
        let bytes =
            "{\"type\":\"Execute\",\"id\":1,\"command\":\"α\"}\n{\"type\":\"Ping\",\"id\":2}\n"
                .as_bytes();
        let split = bytes.iter().position(|byte| *byte == 0xce).unwrap() + 1;
        client.write_all(&bytes[..split]).unwrap();
        server.begin_tick();
        assert!(server.poll().is_none());
        client.write_all(&bytes[split..]).unwrap();
        server.begin_tick();
        assert!(
            matches!(server.poll(), Some(IpcRequest::Execute { id: 1, command, .. }) if command == "α")
        );
        assert!(matches!(server.poll(), Some(IpcRequest::Ping { id: 2 })));
        assert!(server.poll().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn malformed_task_ids_receive_correlated_errors_from_raw_frames() {
        use std::os::unix::net::UnixStream;
        let path = crate::tests::socket_path("invalid-id");
        let mut server = IpcServer::bind(&path).unwrap();
        let mut client = UnixStream::connect(&path).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        for (request, expected) in [
            (
                r#"{"type":"GetTask","id":7,"task_id":"bad"}"#,
                serde_json::json!({"type":"Task", "id":7, "result":{"Err":"invalid_id"}}),
            ),
            (
                r#"{"type":"CancelTask","id":8,"task_id":42}"#,
                serde_json::json!({"type":"TaskCancellation", "id":8, "result":{"Err":"invalid_id"}}),
            ),
        ] {
            writeln!(client, "{request}").unwrap();
            server.begin_tick();
            assert!(server.poll().is_none());
            server.end_tick();
            let mut bytes = [0_u8; 1024];
            let count = client.read(&mut bytes).unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes[..count]).unwrap(),
                expected
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn write_half_close_preserves_request_and_deferred_reply() {
        use std::net::Shutdown;
        use std::os::unix::net::UnixStream;
        let path = crate::tests::socket_path("half-close");
        let mut server = IpcServer::bind(&path).unwrap();
        let mut client = UnixStream::connect(&path).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        client.write_all(b"{\"type\":\"Ping\",\"id\":7}\n").unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        server.begin_tick();
        assert!(matches!(server.poll(), Some(IpcRequest::Ping { id: 7 })));
        let generation = server.connection_generation();
        server.set_pending_replies(1);
        server.end_tick();
        server.begin_tick();
        assert_eq!(server.connection_generation(), generation);
        server.send(IpcResponse::Pong { id: 7 }).unwrap();
        server.set_pending_replies(0);
        server.end_tick();
        let mut bytes = Vec::new();
        client.read_to_end(&mut bytes).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::json!({"type":"Pong", "id":7})
        );
        assert!(server.connection_generation().is_none());
    }

    #[test]
    fn schema_error_requires_a_recoverable_numeric_correlation_id() {
        assert!(matches!(
            invalid_request_response(
                br#"{"type":"Execute","id":9,"command":false}"#,
                "bad command"
            ),
            Some(IpcResponse::Error { id: 9, .. })
        ));
        assert!(invalid_request_response(br#"{"type":"GetTask","id":7"#, "invalid JSON").is_none());
        assert!(invalid_request_response(
            br#"{"type":"GetTask","id":"7","task_id":"bad"}"#,
            "invalid id"
        )
        .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn reconnect_discards_unwritten_frames_from_previous_connection() {
        use std::os::unix::net::UnixStream;
        let path = crate::tests::socket_path("frames");
        let mut server = IpcServer::bind(&path).unwrap();
        let first = UnixStream::connect(&path).unwrap();
        server.begin_tick();
        let generation = server.connection_generation();
        server.send(IpcResponse::Pong { id: 1 }).unwrap();
        drop(first);
        server.begin_tick();
        server.end_tick();
        assert!(server.connection_generation().is_none());
        let mut second = UnixStream::connect(&path).unwrap();
        second.set_nonblocking(true).unwrap();
        server.begin_tick();
        assert_ne!(server.connection_generation(), generation);
        server.end_tick();
        let mut bytes = [0_u8; 64];
        assert_eq!(
            second.read(&mut bytes).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        server.send(IpcResponse::Pong { id: 2 }).unwrap();
        server.end_tick();
        let count = second.read(&mut bytes).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes[..count]).unwrap(),
            serde_json::json!({"type":"Pong", "id":2})
        );
    }

    #[cfg(unix)]
    #[test]
    fn half_closed_client_keeps_slot_until_pending_and_buffered_replies_drain() {
        use std::net::Shutdown;
        use std::os::unix::net::UnixStream;
        let path = crate::tests::socket_path("half-close-slot");
        let mut server = IpcServer::bind(&path).unwrap();
        let mut first = UnixStream::connect(&path).unwrap();
        first
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        first.write_all(b"{\"type\":\"Ping\",\"id\":7}\n").unwrap();
        first.shutdown(Shutdown::Write).unwrap();
        server.begin_tick();
        assert!(matches!(server.poll(), Some(IpcRequest::Ping { id: 7 })));
        let generation = server.connection_generation();
        server.set_pending_replies(1);
        for buffered in [false, true] {
            if buffered {
                server.send(IpcResponse::Pong { id: 7 }).unwrap();
                server.set_pending_replies(0);
            }
            let mut competing = UnixStream::connect(&path).unwrap();
            competing
                .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                .unwrap();
            server.begin_tick();
            assert_eq!(server.connection_generation(), generation);
            let mut rejection = Vec::new();
            competing.read_to_end(&mut rejection).unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&rejection).unwrap()["type"],
                "Error"
            );
        }
        server.end_tick();
        let mut reply = Vec::new();
        first.read_to_end(&mut reply).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&reply).unwrap(),
            serde_json::json!({"type":"Pong", "id":7})
        );
        assert!(server.connection_generation().is_none());
    }

    struct BackpressuredWriter {
        bytes: Vec<u8>,
        blocked: bool,
    }

    impl Write for BackpressuredWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.blocked {
                self.blocked = false;
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            self.blocked = true;
            let count = bytes.len().min(997);
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn large_reply_framing_survives_partial_writes_and_backpressure() {
        let response = IpcResponse::Value {
            id: 1,
            value: serde_json::json!("x".repeat(64 * 1024)),
        };
        let mut first = serde_json::to_vec(&response).unwrap();
        first.push(b'\n');
        let second = b"{\"type\":\"Pong\",\"id\":2}\n".to_vec();
        let expected = [first.as_slice(), second.as_slice()].concat();
        let mut frames = VecDeque::from([first, second]);
        let mut offset = 0;
        let mut remaining = expected.len();
        let mut writer = BackpressuredWriter {
            bytes: Vec::new(),
            blocked: false,
        };
        for _ in 0..200 {
            flush_output(
                &mut writer,
                &mut frames,
                &mut offset,
                &mut remaining,
                IO_BYTES_PER_TICK,
            )
            .unwrap();
            if frames.is_empty() {
                break;
            }
        }
        assert!(frames.is_empty());
        assert_eq!(remaining, 0);
        assert_eq!(offset, 0);
        assert_eq!(writer.bytes, expected);
    }

    #[test]
    fn flushing_bounds_frame_count_per_tick() {
        let mut frames = VecDeque::from(vec![vec![b'\n']; OUTPUT_FRAMES_PER_TICK + 1]);
        let mut writer = Vec::new();
        let mut offset = 0;
        let mut remaining = frames.len();
        flush_output(
            &mut writer,
            &mut frames,
            &mut offset,
            &mut remaining,
            IO_BYTES_PER_TICK,
        )
        .unwrap();
        assert_eq!(writer.len(), OUTPUT_FRAMES_PER_TICK);
        assert_eq!(remaining, 1);
        assert_eq!(frames.len(), 1);
    }

    struct FakeSocketCleanup {
        result: Cell<Option<std::io::ErrorKind>>,
    }

    impl FakeSocketCleanup {
        fn success() -> Self {
            Self {
                result: Cell::new(None),
            }
        }

        fn error(kind: std::io::ErrorKind) -> Self {
            Self {
                result: Cell::new(Some(kind)),
            }
        }
    }

    impl SocketCleanup for FakeSocketCleanup {
        fn remove_file(&self, _path: &Path) -> std::io::Result<()> {
            match self.result.get() {
                Some(kind) => Err(std::io::Error::new(kind, "fake cleanup failure")),
                None => Ok(()),
            }
        }
    }

    #[test]
    fn prepare_socket_path_accepts_removed_socket() {
        prepare_socket_path(
            Path::new("/tmp/patinae.sock"),
            &FakeSocketCleanup::success(),
        )
        .expect("removed socket should be accepted");
    }

    #[test]
    fn prepare_socket_path_accepts_missing_socket() {
        prepare_socket_path(
            Path::new("/tmp/patinae.sock"),
            &FakeSocketCleanup::error(std::io::ErrorKind::NotFound),
        )
        .expect("missing stale socket should be accepted");
    }

    #[test]
    fn prepare_socket_path_returns_cleanup_failure() {
        let err = prepare_socket_path(
            Path::new("/tmp/patinae.sock"),
            &FakeSocketCleanup::error(std::io::ErrorKind::PermissionDenied),
        )
        .expect_err("cleanup failure should stop bind");

        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }
}
