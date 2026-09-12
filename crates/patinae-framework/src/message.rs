//! Unified message bus for inter-component communication
//!
//! All GUI components, plugins, and async tasks communicate through a single
//! [`AppMessage`] type and a per-frame [`MessageBus`] queue.

use serde::{Deserialize, Serialize};

/// Unified application message.
///
/// Domain operations (object management, representations, coloring, camera,
/// selections, movie) are represented as `ExecuteCommand` strings, keeping
/// the message enum focused on GUI-level concerns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppMessage {
    // =====================================================================
    // Command Execution
    // =====================================================================
    /// Execute a command string from the command line, sequence viewer,
    /// viewport click, IPC, or any other source.
    ExecuteCommand { command: String, silent: bool },

    // =====================================================================
    // Output / Notifications
    // =====================================================================
    /// Print an informational message to the output log.
    PrintInfo(String),
    /// Print a warning message to the output log.
    PrintWarning(String),
    /// Print an error message to the output log.
    PrintError(String),
    /// Echo a command to the output log.
    PrintCommand(String),
    /// Print a timing message to the output log.
    PrintTiming(String),
    /// Clear the output log.
    PrintClear,
    /// Print a typed message with an explicit presentation format.
    PrintOutput(patinae_cmd::OutputMessage),

    /// Record a file opened successfully by a command.
    RecordRecentFile { path: String, command: String },

    // =====================================================================
    // System
    // =====================================================================
    /// Request a window redraw.
    RequestRedraw,
    /// Request application quit.
    Quit,
    /// Show the application window (make visible).
    ShowWindow,
    /// Hide the application window (make invisible).
    HideWindow,
    /// Focus a panel by id.
    FocusPanel(String),

    // =====================================================================
    // Layout / Component System
    // =====================================================================
    /// Toggle a panel's expanded/collapsed state by component ID.
    TogglePanel(String),
    /// Show a panel (make visible and expanded).
    ShowPanel(String),
    /// Hide a panel completely (not even rendered as a collapsed tab).
    HidePanel(String),
    /// Detach a docked panel into a floating window.
    FloatPanel(String),
    /// Dock a floating panel back to its original slot.
    DockPanel(String),
    /// Switch the active tab in a tabbed panel group.
    ActivateTab(String),

    // =====================================================================
    // Viewport Image Overlay
    // =====================================================================
    /// Display an RGBA image overlay in the viewport.
    /// Plugins and commands can use this to show rendered or computed images.
    SetViewportImage {
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
    /// Clear the viewport image overlay.
    ClearViewportImage,

    // =====================================================================
    // Extensibility (Plugins)
    // =====================================================================
    /// Custom event for plugins. `topic` identifies the message type,
    /// `payload` is MessagePack-serialized data (use [`topics`](crate::topics) helpers).
    Custom { topic: String, payload: Vec<u8> },
}

/// Per-frame message queue with two-phase buffering.
///
/// Components write to the outbox during the egui frame. After the frame,
/// the app drains the outbox and dispatches messages. On the next frame,
/// drained messages become the inbox (readable by components if needed).
///
/// # Lifecycle per frame
///
/// 1. `begin_frame()` — moves outbox -> inbox
/// 2. Components read inbox, write to outbox via `send()`
/// 3. `drain_outbox()` — app processes new messages
pub struct MessageBus {
    inbox: Vec<AppMessage>,
    outbox: Vec<AppMessage>,
}

impl MessageBus {
    /// Create an empty bus.
    pub fn new() -> Self {
        Self {
            inbox: Vec::with_capacity(8),
            outbox: Vec::with_capacity(8),
        }
    }

    /// Start a new frame: moves outbox -> inbox for component reading.
    pub fn begin_frame(&mut self) {
        self.inbox.clear();
        std::mem::swap(&mut self.inbox, &mut self.outbox);
    }

    /// Send a message (writes to outbox).
    #[inline]
    pub fn send(&mut self, msg: AppMessage) {
        self.outbox.push(msg);
    }

    /// Read the inbox (messages from the previous frame).
    pub fn inbox(&self) -> &[AppMessage] {
        &self.inbox
    }

    /// Drain all outbox messages for dispatch.
    pub fn drain_outbox(&mut self) -> Vec<AppMessage> {
        std::mem::take(&mut self.outbox)
    }

    /// Inject a system event directly into the outbox.
    pub fn inject(&mut self, msg: AppMessage) {
        self.outbox.push(msg);
    }

    /// Whether the outbox has pending messages.
    pub fn has_pending(&self) -> bool {
        !self.outbox.is_empty()
    }

    // =================================================================
    // Convenience methods
    // =================================================================

    /// Send an `ExecuteCommand` message.
    pub fn execute_command(&mut self, cmd: impl Into<String>) {
        self.send(AppMessage::ExecuteCommand {
            command: cmd.into(),
            silent: false,
        });
    }

    /// Send an `ExecuteCommand` message with `silent: true` (no output echo).
    ///
    /// Use for high-frequency operations (e.g., frame slider dragging) where
    /// command echo to the output log would be noisy.
    pub fn execute_command_silent(&mut self, cmd: impl Into<String>) {
        self.send(AppMessage::ExecuteCommand {
            command: cmd.into(),
            silent: true,
        });
    }

    /// Send a typed output message with its format and severity.
    pub fn print_message(&mut self, message: patinae_cmd::OutputMessage) {
        self.send(AppMessage::PrintOutput(message));
    }

    /// Print an informational Markdown message.
    pub fn print_markdown(&mut self, msg: impl Into<String>) {
        self.print_message(patinae_cmd::OutputMessage::markdown(msg));
    }

    /// Send a `PrintInfo` message.
    pub fn print_info(&mut self, msg: impl Into<String>) {
        self.send(AppMessage::PrintInfo(msg.into()));
    }

    /// Send a `PrintWarning` message.
    pub fn print_warning(&mut self, msg: impl Into<String>) {
        self.send(AppMessage::PrintWarning(msg.into()));
    }

    /// Send a `PrintError` message.
    pub fn print_error(&mut self, msg: impl Into<String>) {
        self.send(AppMessage::PrintError(msg.into()));
    }

    /// Send a `PrintTiming` message.
    pub fn print_timing(&mut self, msg: impl Into<String>) {
        self.send(AppMessage::PrintTiming(msg.into()));
    }

    /// Send a `PrintClear` message.
    pub fn print_clear(&mut self) {
        self.send(AppMessage::PrintClear);
    }

    /// Send a `RecordRecentFile` message.
    pub fn record_recent_file(&mut self, path: impl Into<String>, command: impl Into<String>) {
        self.send(AppMessage::RecordRecentFile {
            path: path.into(),
            command: command.into(),
        });
    }

    /// Send a `RequestRedraw` message.
    pub fn request_redraw(&mut self) {
        self.send(AppMessage::RequestRedraw);
    }

    /// Send a `SetViewportImage` message to display an RGBA image in the viewport.
    pub fn set_viewport_image(&mut self, data: Vec<u8>, width: u32, height: u32) {
        self.send(AppMessage::SetViewportImage {
            data,
            width,
            height,
        });
    }

    /// Send a `ClearViewportImage` message to remove the viewport image overlay.
    pub fn clear_viewport_image(&mut self) {
        self.send(AppMessage::ClearViewportImage);
    }
}

impl Default for MessageBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_two_phase_buffering() {
        let mut bus = MessageBus::new();

        // Frame 1: send a message
        bus.send(AppMessage::RequestRedraw);
        assert!(bus.has_pending());
        assert!(bus.inbox().is_empty());

        // Drain outbox (app dispatch)
        let messages = bus.drain_outbox();
        assert_eq!(messages.len(), 1);
        assert!(!bus.has_pending());

        // Frame 2: begin_frame moves nothing (outbox was drained)
        bus.begin_frame();
        assert!(bus.inbox().is_empty());
    }

    #[test]
    fn test_inbox_carries_over() {
        let mut bus = MessageBus::new();

        // Send messages but don't drain
        bus.send(AppMessage::PrintInfo("hello".into()));
        bus.send(AppMessage::RequestRedraw);

        // Begin next frame: outbox -> inbox
        bus.begin_frame();
        assert_eq!(bus.inbox().len(), 2);
        assert!(!bus.has_pending());
    }

    #[test]
    fn test_convenience_methods() {
        let mut bus = MessageBus::new();

        bus.execute_command("load test.pdb");
        bus.print_info("loaded");
        bus.print_timing("1.0 ms");
        bus.print_clear();
        bus.request_redraw();

        let messages = bus.drain_outbox();
        assert_eq!(messages.len(), 5);

        assert!(
            matches!(&messages[0], AppMessage::ExecuteCommand { command, silent: false } if command == "load test.pdb")
        );
        assert!(matches!(&messages[1], AppMessage::PrintInfo(msg) if msg == "loaded"));
        assert!(matches!(&messages[2], AppMessage::PrintTiming(msg) if msg == "1.0 ms"));
        assert!(matches!(&messages[3], AppMessage::PrintClear));
        assert!(matches!(&messages[4], AppMessage::RequestRedraw));
    }
}
