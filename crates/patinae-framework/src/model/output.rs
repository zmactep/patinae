//! Output Model
//!
//! Pure domain model for the output/log message buffer.
//! No egui dependency — testable, serializable, headless-compatible.

use std::collections::VecDeque;

use patinae_cmd::{MessageKind, OutputFormat};

/// Maximum number of lines in the output buffer
const MAX_OUTPUT_LINES: usize = 1000;

/// Kind of output message
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// Normal output
    Normal,
    /// Info/status message
    Info,
    /// Warning message
    Warning,
    /// Error message
    Error,
    /// Command echo
    Command,
    /// Wall-clock timing badge
    Timing,
}

/// Output message type for colored display
#[derive(Debug, Clone)]
pub struct OutputMessage {
    /// The message text
    pub text: String,
    /// Message type for coloring
    pub kind: OutputKind,
    /// Presentation format, independent of severity.
    pub format: OutputFormat,
}

impl OutputMessage {
    /// Create a normal message
    pub fn normal(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: OutputKind::Normal,
            format: OutputFormat::Text,
        }
    }

    /// Create an info message
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: OutputKind::Info,
            format: OutputFormat::Text,
        }
    }

    /// Create a warning message
    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: OutputKind::Warning,
            format: OutputFormat::Text,
        }
    }

    /// Create an error message
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: OutputKind::Error,
            format: OutputFormat::Text,
        }
    }

    /// Create a command echo message
    pub fn command(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: OutputKind::Command,
            format: OutputFormat::Text,
        }
    }

    /// Create a timing badge message
    pub fn timing(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: OutputKind::Timing,
            format: OutputFormat::Text,
        }
    }
}

impl From<patinae_cmd::OutputMessage> for OutputMessage {
    fn from(message: patinae_cmd::OutputMessage) -> Self {
        Self {
            text: message.text,
            kind: match message.kind {
                MessageKind::Info => OutputKind::Info,
                MessageKind::Warning => OutputKind::Warning,
                MessageKind::Error => OutputKind::Error,
            },
            format: message.format,
        }
    }
}

/// Output buffer model for log/message display
#[derive(Debug)]
pub struct OutputModel {
    /// Output messages buffer (circular)
    pub buffer: VecDeque<OutputMessage>,
    /// Whether to auto-scroll output to bottom
    pub auto_scroll: bool,
    /// Monotonic generation bumped on output mutations.
    generation: u64,
}

impl Default for OutputModel {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputModel {
    /// Create a new output model with default values
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::with_capacity(MAX_OUTPUT_LINES),
            auto_scroll: true,
            generation: 0,
        }
    }

    /// Return the output change generation.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Add an output message to the buffer
    pub fn add(&mut self, message: OutputMessage) {
        self.buffer.push_back(message);
        self.generation = self.generation.wrapping_add(1);
        // Trim if over capacity
        while self.buffer.len() > MAX_OUTPUT_LINES {
            self.buffer.pop_front();
        }
    }

    /// Add a normal output line
    pub fn print(&mut self, text: impl Into<String>) {
        self.add(OutputMessage::normal(text));
    }

    /// Add an info output line
    pub fn print_info(&mut self, text: impl Into<String>) {
        self.add(OutputMessage::info(text));
    }

    /// Add a warning output line
    pub fn print_warning(&mut self, text: impl Into<String>) {
        self.add(OutputMessage::warning(text));
    }

    /// Add an error output line
    pub fn print_error(&mut self, text: impl Into<String>) {
        self.add(OutputMessage::error(text));
    }

    /// Add a command echo line
    pub fn print_command(&mut self, text: impl Into<String>) {
        self.add(OutputMessage::command(text));
    }

    /// Add a timing badge line
    pub fn print_timing(&mut self, text: impl Into<String>) {
        self.add(OutputMessage::timing(text));
    }

    /// Clear the output buffer
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.generation = self.generation.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_increments_when_message_added() {
        let mut output = OutputModel::new();
        let generation = output.generation();

        output.print_info("line");

        assert_eq!(output.generation(), generation.wrapping_add(1));
        assert_eq!(output.buffer.len(), 1);
    }

    #[test]
    fn clear_increments_generation_when_buffer_is_empty() {
        let mut output = OutputModel::new();
        let generation = output.generation();

        output.clear();

        assert_eq!(output.generation(), generation.wrapping_add(1));
        assert!(output.buffer.is_empty());
    }

    #[test]
    fn generation_increments_when_capacity_trim_keeps_length() {
        let mut output = OutputModel::new();
        for i in 0..MAX_OUTPUT_LINES {
            output.print_info(i.to_string());
        }
        let generation = output.generation();
        let len = output.buffer.len();
        let first = output
            .buffer
            .front()
            .expect("output should be full")
            .text
            .clone();

        output.print_info("overflow");

        assert_eq!(output.buffer.len(), len);
        assert_eq!(output.generation(), generation.wrapping_add(1));
        assert_ne!(
            output
                .buffer
                .front()
                .expect("output should not be empty")
                .text,
            first
        );
        assert_eq!(
            output
                .buffer
                .back()
                .expect("output should not be empty")
                .text,
            "overflow"
        );
    }
}
