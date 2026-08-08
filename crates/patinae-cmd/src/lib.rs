//! Command system.
//!
//! This crate provides the command parsing, registration, and execution layer.
//!
//! # Overview
//!
//! The command system allows executing text commands, supporting:
//! - Positional and named arguments
//! - Selection expressions
//! - Script (.pml) file execution
//! - Command history
//!
//! # Example
//!
//! ```rust,ignore
//! use patinae_cmd::CommandExecutor;
//! use patinae_scene::Viewer;
//!
//! let mut viewer = Viewer::new();
//! let mut executor = CommandExecutor::new();
//!
//! // Execute commands
//! executor.do_(&mut viewer, "load protein.pdb")?;
//! executor.do_(&mut viewer, "show cartoon")?;
//! executor.do_(&mut viewer, "color green, chain A")?;
//! executor.do_(&mut viewer, "zoom")?;
//! ```
//!
//! # Architecture
//!
//! The command system consists of several components:
//!
//! - **Parser**: Parses command strings into structured `ParsedCommand` objects
//! - **Command trait**: Interface for implementing commands
//! - **CommandRegistry**: Maps command names to implementations
//! - **CommandExecutor**: Dispatches and executes commands
//! - **ScriptEngine**: Executes .pml script files

mod args;
mod command;
pub mod commands;
mod dynamic;
mod error;
mod executor;
pub mod helpers;
mod history;
mod parser;
mod script;
mod setting_access;

// Re-export main types
pub use args::{ArgValue, ParsedCommand};
pub use command::{
    ArgHint, AsyncCommandRequest, AsyncCommandSink, Command, CommandAction, CommandContext,
    CommandRegistry, CommandRuntimeRequirements, CommandSource, DynamicSettingEntry,
    DynamicSettingRegistry, FetchFormatCode, FetchRequest, FormatHandler, LoadedPluginCapability,
    MessageKind, OutputMessage, PluginReaderFn, PluginWriterFn, ScriptHandler, ViewerLike,
};
#[doc(inline)]
pub use commands::display::{
    execute_label_request, LabelExpression, LabelOutcome, LabelRequest, LabelTarget,
};
#[doc(inline)]
pub use commands::measuring::{
    execute_measurement_request, measurement_kind_for_count, MeasurementOutcome,
    MeasurementRequest, MeasurementTarget,
};
pub use dynamic::{DynamicCommand, DynamicCommandInvocation};
pub use error::{CmdError, CmdResult, ParseError};
pub use executor::{CommandExecutor, CommandOutput};
pub use history::CommandHistory;
pub use parser::{join_continued_lines, parse_command, parse_commands};
pub use script::ScriptEngine;
pub use setting_access::{ResolvedSetting, SettingSource};

/// Represents one native annotation mutation request.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationRequest {
    /// Creates or appends a measurement.
    Measurement(MeasurementRequest),
    /// Creates or appends atom labels.
    Label(LabelRequest),
}

/// Describes one successfully applied annotation request.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationOutcome {
    /// Measurement mutation details.
    Measurement(MeasurementOutcome),
    /// Label mutation details.
    Label(LabelOutcome),
}

impl AnnotationOutcome {
    /// Returns the object created or appended by the request.
    pub fn object_name(&self) -> &str {
        match self {
            Self::Measurement(outcome) => &outcome.object_name,
            Self::Label(outcome) => &outcome.object_name,
        }
    }
}

/// Validates and applies one native annotation request.
///
/// # Errors
///
/// Returns the underlying command error without partially mutating an
/// annotation object.
pub fn execute_annotation_request(
    viewer: &mut dyn ViewerLike,
    request: &AnnotationRequest,
) -> CmdResult<AnnotationOutcome> {
    match request {
        AnnotationRequest::Measurement(request) => {
            execute_measurement_request(viewer, request).map(AnnotationOutcome::Measurement)
        }
        AnnotationRequest::Label(request) => {
            execute_label_request(viewer, request).map(AnnotationOutcome::Label)
        }
    }
}

/// Prelude for convenient imports
pub mod prelude {
    pub use crate::args::{ArgValue, ParsedCommand};
    pub use crate::command::{
        ArgHint, AsyncCommandRequest, AsyncCommandSink, Command, CommandAction, CommandContext,
        CommandRegistry, CommandRuntimeRequirements, CommandSource, FetchFormatCode, FetchRequest,
        FormatHandler, LoadedPluginCapability, MessageKind, OutputMessage, PluginReaderFn,
        PluginWriterFn, ScriptHandler, ViewerLike,
    };
    pub use crate::error::{CmdError, CmdResult};
    pub use crate::executor::CommandExecutor;
    pub use crate::parser::{parse_command, parse_commands};
    pub use crate::setting_access::{ResolvedSetting, SettingSource};
}
