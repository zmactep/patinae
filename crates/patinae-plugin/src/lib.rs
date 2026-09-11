//! Plugin SDK.
//!
//! This crate provides everything a plugin author needs to extend the application
//! with new commands, declarative UI panels, and message handlers.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use patinae_plugin::prelude::*;
//!
//! patinae_plugin! {
//!     name: "my-plugin",
//!     version: "0.1.0",
//!     description: "My awesome plugin",
//!     commands: [MyCommand],
//! }
//! ```
//!
//! # Architecture
//!
//! Plugins are compiled as `cdylib` shared libraries and loaded at runtime
//! by the host application. The `patinae_plugin!` macro generates a C-compatible
//! entry point that the host discovers via `libloading`.

pub mod ffi;
pub mod macros;
pub mod registrar;
pub mod wire;

/// Shared task identities, snapshots, outcomes, and query options.
pub use patinae_cmd::tasks;

// Re-export log so the patinae_plugin! macro can reference it without
// plugins needing an explicit `log` dependency.
pub use log;

/// Hidden re-exports used by `define_plugin_settings!` macro internals.
/// Not part of the public API.
#[doc(hidden)]
pub mod __private {
    pub use patinae_settings::{SettingType, SettingValue, SideEffectCategory};
}

/// Convenient re-exports for plugin authors.
///
/// A single `use patinae_plugin::prelude::*;` gives access to all types
/// needed to implement commands, panels, and message handlers.
pub mod prelude {
    pub use crate::registrar::TaskQueryResult;
    pub use crate::tasks::{TaskId, TaskListRequest, TaskOutcomeStatus, TaskSnapshot, TaskState};
    pub use patinae_cmd::{
        ArgumentSyntax, AsyncCommandRequest, CommandReply, PluginTaskRequest, TaskInvocation,
    };
    // Framework types
    pub use patinae_framework::atom_stream::{
        AtomChunk, AtomColumn, AtomRow, AtomRowKey, AtomStreamMode, AtomStreamRequest,
        AtomStreamScope, AtomValue,
    };
    pub use patinae_framework::component::SharedContext;
    pub use patinae_framework::message::{AppMessage, MessageBus};
    pub use patinae_framework::plugin_ui::{
        PanelAction, PanelButton, PanelColumn, PanelControl, PanelControlNode, PanelDescriptor,
        PanelEvent, PanelEventKind, PanelGroup, PanelOption, PanelPlacement, PanelRow,
        PanelRuntimeRequirements, PanelSnapshot, PanelTextArea, PanelTextHighlight, PanelTextStyle,
        PanelValue, PluginPanel,
    };
    pub use patinae_framework::topics::{
        custom_action, publish, subscribe, SaveFileRequest, SAVE_FILE_REQUEST_TOPIC,
    };

    // Command system
    pub use patinae_cmd::command_help;
    pub use patinae_cmd::{
        ArgHint, CmdError, CmdResult, Command, CommandContext, CommandRuntimeRequirements,
        MessageKind, OutputMessage, ParsedCommand, ViewerLike,
    };

    // Domain types
    pub use patinae_mol::{Atom, Bond, CoordSet, ObjectMolecule};
    pub use patinae_scene::{
        Camera, GpuAddressMode, GpuBatchCommand, GpuBatchResult, GpuBindGroupDescriptor,
        GpuBindGroupEntry, GpuBindGroupLayoutDescriptor, GpuBindGroupLayoutEntry,
        GpuBindingResource, GpuBindingType, GpuBlendState, GpuBufferBinding, GpuBufferBindingType,
        GpuBufferDescriptor, GpuBufferUsage, GpuCacheStats, GpuCacheStatus, GpuCachedHandle,
        GpuColor, GpuColorLoadOp, GpuColorOperations, GpuColorTargetState, GpuColorWriteMask,
        GpuCompareFunction, GpuComputePipelineDescriptor, GpuDepthLoadOp, GpuDepthOperations,
        GpuDepthStencilState, GpuDeviceLimits, GpuExtent3d, GpuFace, GpuFilterMode,
        GpuFragmentState, GpuFrontFace, GpuHandle, GpuHandleKind, GpuIndexFormat,
        GpuMultisampleState, GpuOrigin3d, GpuPipelineLayoutDescriptor, GpuPrimitiveState,
        GpuPrimitiveTopology, GpuRenderPassColorAttachment, GpuRenderPassCommand,
        GpuRenderPassDepthStencilAttachment, GpuRenderPassDescriptor, GpuRenderPipelineDescriptor,
        GpuSamplerBindingType, GpuSamplerDescriptor, GpuShaderModuleDescriptor, GpuShaderStages,
        GpuStorageTextureAccess, GpuStoreOp, GpuSubmitBatch, GpuTexelCopyBufferLayout,
        GpuTexelCopyTextureInfo, GpuTextureAspect, GpuTextureDescriptor, GpuTextureDimension,
        GpuTextureFormat, GpuTextureSampleType, GpuTextureUsage, GpuTextureViewDescriptor,
        GpuTextureViewDimension, GpuVertexAttribute, GpuVertexBufferLayout, GpuVertexFormat,
        GpuVertexState, GpuVertexStepMode, ObjectRegistry, RenderArtifactBufferDescriptor,
        RenderArtifactBufferRole, RenderArtifactPrimitiveTopology, RenderArtifactRepDescriptor,
        RenderArtifactRepKind, RenderArtifactSnapshotDescriptor, SelectionManager,
    };
    pub use patinae_select::select;

    // Plugin API
    pub use crate::registrar::{
        parse_key_string, CommandResult, DynamicSettingDescriptor, DynamicSettingStore,
        FormatHandler, HotkeyCallback, KeyBinding, KeyCode, MessageHandler, PluginKeyAction,
        PluginMetadata, PluginReaderFn, PluginRegistrar, PluginWriterFn, PollContext,
        SharedSettingStore,
    };
    // Settings side effects (needed for define_plugin_settings! macro)
    pub use patinae_settings::SideEffectCategory;
    // patinae_plugin! macro is auto-exported via #[macro_export]
    // define_plugin_settings! macro is auto-exported via #[macro_export]
}
