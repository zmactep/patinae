//! Versioned runtime wire payloads.
//!
//! Dynamic plugins exchange rich runtime data with the host as MessagePack
//! bytes. This module owns those DTOs so the FFI boundary stays limited to
//! pointer-length byte views and opaque handles.

use std::io::Read;
use std::sync::{Arc, RwLock};

use patinae_cmd::{
    CommandAction, DynamicCommandInvocation, DynamicSettingRegistry, OutputMessage, ParsedCommand,
};
use patinae_framework::atom_stream::{AtomChunk, AtomStreamRequest};
use patinae_framework::message::AppMessage;
use patinae_framework::plugin_ui::{PanelAction, PanelEvent, PanelSnapshot};
use patinae_mol::ObjectMolecule;
use patinae_render::{
    DisplayedGeometry, DisplayedMaterial, DisplayedMesh, DisplayedMeshVertex,
    DisplayedObjectGeometry, DisplayedPrimitive, ObjectId, RepKind, TraceCylinder,
    TraceGeometryChunk, TraceLineSegment, TraceMaterial, TracePointSample, TraceSphere,
    TraceTriangle,
};
use patinae_scene::{
    Camera, GpuBatchResult, GpuBindGroupDescriptor, GpuBindGroupLayoutDescriptor,
    GpuBufferDescriptor, GpuCacheStats, GpuCachedHandle, GpuComputePipelineDescriptor, GpuHandle,
    GpuPipelineLayoutDescriptor, GpuRenderPipelineDescriptor, GpuSamplerDescriptor,
    GpuShaderModuleDescriptor, GpuSubmitBatch, GpuTextureDescriptor, GpuTextureViewDescriptor,
    LabelObjectView, MovieStateSnapshot, RenderArtifactSnapshotDescriptor, SceneView, Session,
    ViewportImage,
};
use patinae_settings::{
    DynamicSettingDescriptor, DynamicSettingStore, SettingType, SettingValue, SideEffectCategory,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Runtime wire version for MessagePack DTOs.
pub const RUNTIME_WIRE_VERSION: u32 = 13;

/// Maximum MessagePack payload copied across the runtime ABI.
pub const MAX_WIRE_PAYLOAD_LEN: usize = 64 * 1024 * 1024;

/// Magic header for displayed-geometry spool files.
///
/// Spools are process-local handoff files used when geometry is too large for
/// one ABI payload. Keeping a versioned header makes stale or mismatched files
/// fail before any chunk decoding starts.
pub const DISPLAYED_GEOMETRY_SPOOL_MAGIC: &[u8; 8] = b"PTGEO01\0";

mod displayed_geometry;
mod trace_geometry;

pub use displayed_geometry::{
    displayed_geometry_chunk_from_wire, displayed_geometry_from_wire, displayed_geometry_to_wire,
    displayed_object_geometry_to_wire, displayed_primitive_to_wire,
};
pub use trace_geometry::{trace_geometry_chunk_from_wire, trace_geometry_chunk_to_wire};

/// Serialized dynamic setting state.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireDynamicSetting {
    /// Setting descriptor metadata.
    pub descriptor: WireDynamicSettingDescriptor,
    /// Explicit global value, if one has been set.
    pub value: Option<SettingValue>,
}

/// Serialized dynamic setting descriptor.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireDynamicSettingDescriptor {
    /// Setting name.
    pub name: String,
    /// Value type.
    pub setting_type: SettingType,
    /// Default value.
    pub default: SettingValue,
    /// Optional minimum value.
    pub min: Option<f32>,
    /// Optional maximum value.
    pub max: Option<f32>,
    /// Named value hints.
    pub value_hints: Vec<(String, SettingValue)>,
    /// Side effects for host invalidation.
    pub side_effects: Vec<SideEffectCategory>,
    /// Whether object-specific values are accepted.
    pub object_overridable: bool,
}

/// Lightweight identity for a viewport image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireViewportImageSummary {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Image byte length.
    pub len: usize,
    /// Host-computed identity for change detection.
    pub signature: u64,
}

/// Lightweight host state used by per-frame poll callbacks.
#[derive(Clone, Serialize, Deserialize)]
pub struct WirePollSharedInput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Scene generation counter.
    pub scene_generation: u64,
    /// Loaded object names.
    pub object_names: Vec<String>,
    /// Current camera state.
    pub camera: Camera,
    /// Current movie state.
    pub movie: MovieStateSnapshot,
    /// Current global settings.
    pub settings: patinae_settings::Settings,
    /// Current background color.
    pub clear_color: [f32; 3],
    /// Viewport image identity without pixel bytes.
    pub viewport_image: Option<WireViewportImageSummary>,
    /// Known command names.
    pub command_names: Vec<String>,
    /// Known setting names.
    pub setting_names: Vec<String>,
    /// Host-owned dynamic setting state.
    pub dynamic_settings: Vec<WireDynamicSetting>,
}

/// Renderer-neutral displayed geometry for runtime command input.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireDisplayedGeometry {
    /// Display objects in render order.
    pub objects: Vec<WireDisplayedObjectGeometry>,
}

/// One renderer-neutral displayed-geometry chunk.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireDisplayedGeometryChunk {
    /// Display objects in render order for this chunk.
    pub objects: Vec<WireDisplayedObjectGeometry>,
}

/// Process-local chunk spool for oversized displayed geometry.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireDisplayedGeometrySpool {
    /// Filesystem path to the length-prefixed chunk stream.
    pub path: String,
    /// Number of chunks written by the host.
    pub chunk_count: usize,
}

/// Compact trace geometry chunk for command-scoped streaming.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireTraceGeometryChunk {
    /// Analytic spheres.
    pub spheres: Vec<WireTraceSphere>,
    /// Analytic cylinders.
    pub cylinders: Vec<WireTraceCylinder>,
    /// Triangle-list mesh primitives.
    pub triangles: Vec<WireTraceTriangle>,
    /// Semantic line samples.
    pub line_segments: Vec<WireTraceLineSegment>,
    /// Semantic point samples.
    pub point_samples: Vec<WireTracePointSample>,
}

/// Compact trace material.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireTraceMaterial {
    /// Final resolved RGBA color.
    pub rgba: [f32; 4],
    /// PyMOL-style transparency.
    pub transparency: f32,
}

/// Compact trace sphere.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireTraceSphere {
    /// Sphere center.
    pub center: [f32; 3],
    /// Sphere radius.
    pub radius: f32,
    /// Resolved material.
    pub material: WireTraceMaterial,
}

/// Compact trace cylinder.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireTraceCylinder {
    /// Cylinder start.
    pub start: [f32; 3],
    /// Cylinder end.
    pub end: [f32; 3],
    /// Cylinder radius.
    pub radius: f32,
    /// Material at the start.
    pub material_start: WireTraceMaterial,
    /// Material at the end.
    pub material_end: WireTraceMaterial,
}

/// Compact trace triangle.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireTraceTriangle {
    /// Triangle vertex positions.
    pub positions: [[f32; 3]; 3],
    /// Triangle vertex normals.
    pub normals: [[f32; 3]; 3],
    /// Pre-reduced material.
    pub material: WireTraceMaterial,
}

/// Semantic line sample.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireTraceLineSegment {
    /// Line start.
    pub start: [f32; 3],
    /// Line end.
    pub end: [f32; 3],
    /// Screen-space width in pixels.
    pub width_px: f32,
    /// Material at the start.
    pub material_start: WireTraceMaterial,
    /// Material at the end.
    pub material_end: WireTraceMaterial,
}

/// Semantic point sample.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireTracePointSample {
    /// Point position.
    pub position: [f32; 3],
    /// Screen-space radius in pixels.
    pub radius_px: f32,
    /// Resolved material.
    pub material: WireTraceMaterial,
}

/// Displayed geometry for one renderer object.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireDisplayedObjectGeometry {
    /// Host renderer object id.
    pub object_id: u32,
    /// Displayed primitives for this object.
    pub primitives: Vec<WireDisplayedPrimitive>,
}

/// A displayed primitive in wire form.
#[derive(Clone, Serialize, Deserialize)]
pub enum WireDisplayedPrimitive {
    /// Non-indexed triangle-list mesh.
    Mesh {
        /// Raw [`RepKind`] tag.
        rep: u8,
        /// Mesh vertices.
        vertices: Vec<WireDisplayedMeshVertex>,
    },
    /// Analytic sphere.
    Sphere {
        /// Raw [`RepKind`] tag.
        rep: u8,
        /// Owning atom id.
        owner_atom_id: u32,
        /// Sphere center.
        center: [f32; 3],
        /// Sphere radius.
        radius: f32,
        /// Resolved material.
        material: WireDisplayedMaterial,
    },
    /// Analytic cylinder.
    Cylinder {
        /// Raw [`RepKind`] tag.
        rep: u8,
        /// Owning atom ids.
        owner_atom_ids: [u32; 2],
        /// Cylinder start.
        start: [f32; 3],
        /// Cylinder end.
        end: [f32; 3],
        /// Cylinder radius.
        radius: f32,
        /// Material at the start.
        material_start: WireDisplayedMaterial,
        /// Material at the end.
        material_end: WireDisplayedMaterial,
    },
    /// Semantic screen-space line.
    LineSegment {
        /// Raw [`RepKind`] tag.
        rep: u8,
        /// Owning atom ids.
        owner_atom_ids: [u32; 2],
        /// Line start.
        start: [f32; 3],
        /// Line end.
        end: [f32; 3],
        /// Line width in pixels.
        width_px: f32,
        /// Material at the start.
        material_start: WireDisplayedMaterial,
        /// Material at the end.
        material_end: WireDisplayedMaterial,
    },
    /// Semantic screen-space point.
    PointSample {
        /// Raw [`RepKind`] tag.
        rep: u8,
        /// Owning atom id.
        owner_atom_id: u32,
        /// Point position.
        position: [f32; 3],
        /// Point radius in pixels.
        radius_px: f32,
        /// Resolved material.
        material: WireDisplayedMaterial,
    },
}

/// Displayed mesh vertex in wire form.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireDisplayedMeshVertex {
    /// World position.
    pub position: [f32; 3],
    /// World normal.
    pub normal: [f32; 3],
    /// Owning atom id.
    pub owner_atom_id: u32,
    /// Resolved material.
    pub material: WireDisplayedMaterial,
    /// Renderer flags.
    pub flags: u32,
}

/// Display material in wire form.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireDisplayedMaterial {
    /// Host-resolved base color.
    pub base_rgba: [f32; 4],
    /// Representation color.
    pub rep_rgba: [f32; 4],
    /// Final display color.
    pub rgba: [f32; 4],
    /// PyMOL-style transparency.
    pub transparency: f32,
}

/// Portable atom property value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireAtomPropertyValue {
    /// String value.
    Str(String),
    /// 32-bit float value.
    F32(f32),
    /// 32-bit integer value.
    I32(i32),
    /// 8-bit integer value.
    I8(i8),
    /// Boolean value.
    Bool(bool),
}

/// Portable atom property change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireAtomPropertyChange {
    /// Object name.
    pub object: String,
    /// Zero-based atom index.
    pub atom_index: u32,
    /// Property changes.
    pub changes: Vec<(String, WireAtomPropertyValue)>,
}

/// Portable viewer-side actions emitted by dynamic plugin polling.
#[derive(Clone, Serialize, Deserialize)]
pub enum WireViewerAction {
    /// Apply atom property changes.
    ApplyAtomPropertyChanges(Vec<WireAtomPropertyChange>),
    /// Store a viewport overlay image.
    SetViewportImage(ViewportImage),
    /// Clear the viewport overlay image.
    ClearViewportImage,
    /// Request a viewport redraw.
    RequestRedraw,
    /// Invalidate plugin panel snapshots.
    RequestPanelUpdate,
}

/// Host-side query requested by a dynamic plugin.
#[derive(Clone, Serialize, Deserialize)]
pub enum WireHostQuery {
    /// Return loaded object names.
    ObjectNames { id: u64 },
    /// Return the current camera view.
    View { id: u64 },
    /// Count atoms matching a selection expression.
    CountAtoms { id: u64, selection: String },
    /// Return full viewport image bytes.
    ViewportImage { id: u64 },
    /// Return one portable label-object inspection snapshot.
    LabelObject {
        /// Correlation id.
        id: u64,
        /// Registry-owned label object name.
        name: String,
    },
    /// Open a host-owned atom stream.
    OpenAtomStream {
        /// Correlation id.
        id: u64,
        /// Stream request.
        request: AtomStreamRequest,
    },
    /// Read rows from a host-owned atom stream.
    ReadAtomStream {
        /// Correlation id.
        id: u64,
        /// Host-owned stream id.
        stream_id: u64,
        /// Maximum rows to return.
        max_rows: usize,
    },
    /// Close a host-owned atom stream.
    CloseAtomStream {
        /// Correlation id.
        id: u64,
        /// Host-owned stream id.
        stream_id: u64,
    },
}

/// Host-side query result delivered on a later poll.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireHostQueryResult {
    /// Correlation id.
    pub id: u64,
    /// Query value or portable error.
    pub result: Result<WireHostQueryValue, String>,
}

/// Host-side query value.
#[derive(Clone, Serialize, Deserialize)]
pub enum WireHostQueryValue {
    /// Loaded object names.
    ObjectNames(Vec<String>),
    /// Current camera view.
    View(SceneView),
    /// Atom count.
    CountAtoms(usize),
    /// Full viewport image bytes.
    ViewportImage(Option<ViewportImage>),
    /// Portable label-object inspection snapshot.
    LabelObject(Option<LabelObjectView>),
    /// Open atom stream metadata.
    AtomStreamOpened(WireAtomStreamOpened),
    /// Atom stream rows.
    AtomStreamChunk(AtomChunk),
    /// Atom stream closed.
    AtomStreamClosed,
}

/// Open atom stream metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireAtomStreamOpened {
    /// Host-owned stream id.
    pub stream_id: u64,
    /// Total rows in the stream.
    pub total_count: usize,
}

impl From<&DynamicSettingDescriptor> for WireDynamicSettingDescriptor {
    fn from(value: &DynamicSettingDescriptor) -> Self {
        Self {
            name: value.name.clone(),
            setting_type: value.setting_type,
            default: value.default.clone(),
            min: value.min,
            max: value.max,
            value_hints: value.value_hints.clone(),
            side_effects: value.side_effects.clone(),
            object_overridable: value.object_overridable,
        }
    }
}

impl From<WireDynamicSettingDescriptor> for DynamicSettingDescriptor {
    fn from(value: WireDynamicSettingDescriptor) -> Self {
        Self {
            name: value.name,
            setting_type: value.setting_type,
            default: value.default,
            min: value.min,
            max: value.max,
            value_hints: value.value_hints,
            side_effects: value.side_effects,
            object_overridable: value.object_overridable,
        }
    }
}

/// Command execution input.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireCommandInput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// MessagePack-encoded [`Session`].
    pub session: Vec<u8>,
    /// Current viewport image snapshot.
    pub viewport_image: Option<ViewportImage>,
    /// Parsed command invocation.
    pub parsed: ParsedCommand,
    /// Whether user-visible output should be suppressed.
    pub quiet: bool,
    /// Current viewport width.
    pub viewport_width: u32,
    /// Current viewport height.
    pub viewport_height: u32,
    /// Host-owned dynamic setting state.
    pub dynamic_settings: Vec<WireDynamicSetting>,
    /// Host-resolved displayed geometry, when requested by the command.
    pub displayed_geometry: Option<WireDisplayedGeometry>,
    /// Host-resolved displayed geometry in process-local chunks.
    pub displayed_geometry_spool: Option<WireDisplayedGeometrySpool>,
}

/// Command-scoped host runtime request.
#[derive(Clone, Serialize, Deserialize)]
pub enum WireCommandRuntimeRequest {
    /// Open a compact trace geometry stream.
    OpenTraceGeometryStream {
        /// Correlation id.
        id: u64,
    },
    /// Read the next compact trace geometry chunk.
    ReadTraceGeometryStream {
        /// Correlation id.
        id: u64,
        /// Host-owned stream id.
        stream_id: u64,
    },
    /// Close a compact trace geometry stream.
    CloseTraceGeometryStream {
        /// Correlation id.
        id: u64,
        /// Host-owned stream id.
        stream_id: u64,
    },
    /// Open a renderer artifact snapshot.
    OpenRenderArtifactSnapshot {
        /// Correlation id.
        id: u64,
    },
    /// Close a renderer artifact snapshot.
    CloseRenderArtifactSnapshot {
        /// Correlation id.
        id: u64,
        /// Host-owned snapshot id.
        snapshot_id: u64,
    },
    /// Query command-scoped GPU device limits.
    GpuDeviceLimits {
        /// Correlation id.
        id: u64,
    },
    /// Create a host-owned GPU buffer.
    GpuCreateBuffer {
        /// Correlation id.
        id: u64,
        /// Portable buffer descriptor.
        descriptor: GpuBufferDescriptor,
        /// Optional initial bytes copied by the host.
        initial_data: Option<Vec<u8>>,
    },
    /// Create a host-owned GPU texture.
    GpuCreateTexture {
        /// Correlation id.
        id: u64,
        descriptor: GpuTextureDescriptor,
    },
    /// Create a host-owned GPU texture view.
    GpuCreateTextureView {
        /// Correlation id.
        id: u64,
        descriptor: GpuTextureViewDescriptor,
    },
    /// Create a host-owned GPU sampler.
    GpuCreateSampler {
        /// Correlation id.
        id: u64,
        descriptor: GpuSamplerDescriptor,
    },
    /// Write bytes to a host-owned GPU buffer.
    GpuWriteBuffer {
        /// Correlation id.
        id: u64,
        /// Destination buffer handle.
        buffer: GpuHandle,
        /// Destination byte offset.
        offset: u64,
        /// Bytes copied by the host.
        data: Vec<u8>,
    },
    /// Copy bytes between host-owned GPU buffers.
    GpuCopyBufferToBuffer {
        /// Correlation id.
        id: u64,
        source: GpuHandle,
        source_offset: u64,
        destination: GpuHandle,
        destination_offset: u64,
        size: u64,
    },
    /// Read bytes from a host-owned GPU buffer.
    GpuReadBuffer {
        /// Correlation id.
        id: u64,
        buffer: GpuHandle,
        offset: u64,
        size: u64,
    },
    /// Create a WGSL shader module.
    GpuCreateShaderModule {
        /// Correlation id.
        id: u64,
        descriptor: GpuShaderModuleDescriptor,
    },
    /// Create a bind-group layout.
    GpuCreateBindGroupLayout {
        /// Correlation id.
        id: u64,
        descriptor: GpuBindGroupLayoutDescriptor,
    },
    /// Create a pipeline layout.
    GpuCreatePipelineLayout {
        /// Correlation id.
        id: u64,
        descriptor: GpuPipelineLayoutDescriptor,
    },
    /// Create a compute pipeline.
    GpuCreateComputePipeline {
        /// Correlation id.
        id: u64,
        descriptor: GpuComputePipelineDescriptor,
    },
    /// Create a render pipeline.
    GpuCreateRenderPipeline {
        /// Correlation id.
        id: u64,
        descriptor: GpuRenderPipelineDescriptor,
    },
    /// Create or lease a cached WGSL shader module.
    GpuCreateCachedShaderModule {
        /// Correlation id.
        id: u64,
        descriptor: GpuShaderModuleDescriptor,
    },
    /// Create or lease a cached bind-group layout.
    GpuCreateCachedBindGroupLayout {
        /// Correlation id.
        id: u64,
        descriptor: GpuBindGroupLayoutDescriptor,
    },
    /// Create or lease a cached pipeline layout.
    GpuCreateCachedPipelineLayout {
        /// Correlation id.
        id: u64,
        descriptor: GpuPipelineLayoutDescriptor,
    },
    /// Create or lease a cached compute pipeline.
    GpuCreateCachedComputePipeline {
        /// Correlation id.
        id: u64,
        descriptor: GpuComputePipelineDescriptor,
    },
    /// Create or lease a cached render pipeline.
    GpuCreateCachedRenderPipeline {
        /// Correlation id.
        id: u64,
        descriptor: GpuRenderPipelineDescriptor,
    },
    /// Query persistent GPU cache counters.
    GpuCacheStats {
        /// Correlation id.
        id: u64,
    },
    /// Drop persistent GPU cache entries for the active plugin.
    GpuDropPluginCache {
        /// Correlation id.
        id: u64,
    },
    /// Create a bind group.
    GpuCreateBindGroup {
        /// Correlation id.
        id: u64,
        descriptor: GpuBindGroupDescriptor,
    },
    /// Dispatch a compute pipeline.
    GpuDispatchCompute {
        /// Correlation id.
        id: u64,
        pipeline: GpuHandle,
        bind_groups: Vec<GpuHandle>,
        workgroups: [u32; 3],
    },
    /// Submit one ordered GPU command batch.
    GpuSubmitBatch {
        /// Correlation id.
        id: u64,
        batch: GpuSubmitBatch,
    },
    /// Promote a command-scoped GPU RGBA8 buffer to the native viewport.
    SetViewportGpuImageFromBuffer {
        /// Correlation id.
        id: u64,
        /// Source buffer handle.
        buffer: GpuHandle,
        /// Image width in pixels.
        width: u32,
        /// Image height in pixels.
        height: u32,
    },
    /// Drop host-owned GPU handles before command end.
    GpuDropHandles {
        /// Correlation id.
        id: u64,
        handles: Vec<GpuHandle>,
    },
}

/// Command-scoped host runtime response.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireCommandRuntimeResponse {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Correlation id.
    pub id: u64,
    /// Response value or portable error.
    pub result: Result<WireCommandRuntimeValue, String>,
}

/// Command-scoped host runtime response value.
#[derive(Clone, Serialize, Deserialize)]
pub enum WireCommandRuntimeValue {
    /// Trace geometry stream was opened.
    TraceGeometryOpened(WireTraceGeometryOpened),
    /// Next trace geometry chunk, or `None` at end of stream.
    TraceGeometryChunk(Option<WireTraceGeometryChunk>),
    /// Pre-encoded trace geometry chunk bytes, or `None` at end of stream.
    TraceGeometryChunkBytes(Option<Vec<u8>>),
    /// Trace geometry stream was closed.
    TraceGeometryClosed,
    /// Render artifact snapshot was opened.
    RenderArtifactSnapshotOpened(RenderArtifactSnapshotDescriptor),
    /// Render artifact snapshot was closed.
    RenderArtifactSnapshotClosed,
    /// GPU device limits.
    GpuDeviceLimits(patinae_scene::GpuDeviceLimits),
    /// Newly-created GPU resource handle.
    GpuHandle(GpuHandle),
    /// Command-scoped lease of a cached GPU resource.
    GpuCachedHandle(GpuCachedHandle),
    /// Persistent GPU cache counters.
    GpuCacheStats(GpuCacheStats),
    /// GPU operation completed without a value.
    GpuOk,
    /// Bytes read from a GPU buffer.
    GpuBytes(Vec<u8>),
    /// Result of an ordered GPU command batch.
    GpuBatchResult(GpuBatchResult),
}

/// Open trace geometry stream metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireTraceGeometryOpened {
    /// Host-owned stream id.
    pub stream_id: u64,
}

/// Command execution output.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireCommandOutput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Command result, using a string error for portability.
    pub result: Result<(), String>,
    /// Messages printed by the command.
    pub output: Vec<OutputMessage>,
    /// Host actions requested by the command.
    pub actions: Vec<CommandAction>,
    /// MessagePack-encoded updated [`Session`].
    pub session: Vec<u8>,
    /// Viewport image set by the plugin, if any.
    pub viewport_image: Option<ViewportImage>,
    /// Whether `viewport_image` should be applied to the host CPU viewport slot.
    pub viewport_image_changed: bool,
}

/// Shared host state input for panel and poll callbacks.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireSharedInput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// MessagePack-encoded [`Session`].
    pub session: Vec<u8>,
    /// Scene generation counter.
    pub scene_generation: u64,
    /// Viewport image snapshot.
    pub viewport_image: Option<ViewportImage>,
    /// Known command names.
    pub command_names: Vec<String>,
    /// Known setting names.
    pub setting_names: Vec<String>,
    /// Host-owned dynamic setting state.
    pub dynamic_settings: Vec<WireDynamicSetting>,
}

/// Panel event input.
#[derive(Clone, Serialize, Deserialize)]
pub struct WirePanelEventInput {
    /// Shared host state snapshot.
    pub shared: WireSharedInput,
    /// Frontend event.
    pub event: PanelEvent,
}

/// Panel event output.
#[derive(Clone, Serialize, Deserialize)]
pub struct WirePanelEventOutput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Panel actions emitted by the plugin.
    pub actions: Vec<PanelAction>,
}

/// Panel snapshot output.
#[derive(Clone, Serialize, Deserialize)]
pub struct WirePanelSnapshotOutput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Snapshot to render.
    pub snapshot: PanelSnapshot,
}

/// Command execution result delivered during polling.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireCommandResult {
    /// Correlation ID.
    pub id: u64,
    /// Result of the command.
    pub result: Result<(), String>,
}

/// Deferred command execution request.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireCommandExecRequest {
    /// Correlation ID.
    pub id: u64,
    /// Command text.
    pub command: String,
    /// Whether execution should suppress echo/output.
    pub silent: bool,
}

/// Dynamic command registration request.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireDynCmdRegistration {
    /// Command name.
    pub name: String,
    /// Short description.
    pub description: String,
    /// Usage text.
    pub usage: String,
    /// Argument help text.
    pub arguments: String,
}

/// Portable hotkey action.
#[derive(Clone, Serialize, Deserialize)]
pub enum WireHotkeyAction {
    /// Execute a command string.
    Command(String),
    /// Invoke a dynamic command.
    DynamicCommand { name: String, args: Vec<String> },
    /// Publish a custom message.
    Custom { topic: String, payload: Vec<u8> },
}

/// Portable hotkey registration.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireHotkeyRegistration {
    /// Key string to parse on the host side.
    pub key: String,
    /// Host-owned action.
    pub action: WireHotkeyAction,
}

/// Message handler poll input.
#[derive(Clone, Serialize, Deserialize)]
pub struct WirePollInput {
    /// Lightweight shared host state.
    pub shared: WirePollSharedInput,
    /// Results from previously queued commands.
    pub command_results: Vec<WireCommandResult>,
    /// Results from previously queued host queries.
    pub host_query_results: Vec<WireHostQueryResult>,
    /// Dynamic command invocations since the last poll.
    pub dynamic_invocations: Vec<DynamicCommandInvocation>,
    /// Plugin directories as UTF-8 paths.
    pub plugin_dirs: Vec<String>,
}

/// Message handler poll output.
#[derive(Clone, Serialize, Deserialize)]
pub struct WirePollOutput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Bus messages emitted by the plugin.
    pub messages: Vec<AppMessage>,
    /// Deferred command executions.
    pub command_exec: Vec<WireCommandExecRequest>,
    /// Dynamic command registrations.
    pub dynamic_registrations: Vec<WireDynCmdRegistration>,
    /// Dynamic command unregistrations.
    pub dynamic_unregistrations: Vec<String>,
    /// User notifications.
    pub notifications: Vec<String>,
    /// Hotkey registrations.
    pub hotkey_registrations: Vec<WireHotkeyRegistration>,
    /// Hotkey unregistrations.
    pub hotkey_unregistrations: Vec<String>,
    /// Portable host queries requested by the plugin.
    pub host_queries: Vec<WireHostQuery>,
    /// Portable viewer actions requested by the plugin.
    pub viewer_actions: Vec<WireViewerAction>,
}

/// Message delivery input.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireMessageInput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Broadcast message.
    pub message: AppMessage,
}

/// Message delivery output.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireMessageOutput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Bus messages emitted by the plugin.
    pub messages: Vec<AppMessage>,
}

/// Script handler input.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireScriptInput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Script path.
    pub path: String,
}

/// Script handler output.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireScriptOutput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Script dispatch result.
    pub result: Result<(), String>,
}

/// File format reader input.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireFormatReadInput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// File extension without a dot.
    pub extension: String,
    /// Full file bytes read by the host.
    pub bytes: Vec<u8>,
}

/// File format reader output.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireFormatReadOutput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// Parsed molecules or a portable error.
    pub result: Result<Vec<ObjectMolecule>, String>,
}

/// File format writer input.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireFormatWriteInput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// File extension without a dot.
    pub extension: String,
    /// Molecules selected by the host.
    pub molecules: Vec<ObjectMolecule>,
}

/// File format writer output.
#[derive(Clone, Serialize, Deserialize)]
pub struct WireFormatWriteOutput {
    /// Must equal [`RUNTIME_WIRE_VERSION`].
    pub wire_version: u32,
    /// File bytes to write or a portable error.
    pub result: Result<Vec<u8>, String>,
}

/// Reads displayed-geometry chunks from a process-local spool.
///
/// # Errors
///
/// Returns a string when the file is missing, malformed, exceeds the ABI
/// payload limit for any chunk, or the visitor rejects a decoded chunk.
pub fn for_each_displayed_geometry_spool_chunk(
    spool: &WireDisplayedGeometrySpool,
    mut visitor: impl FnMut(DisplayedGeometry) -> Result<(), String>,
) -> Result<(), String> {
    let mut file = std::fs::File::open(&spool.path)
        .map_err(|error| format!("open displayed geometry spool: {error}"))?;
    let mut magic = [0u8; DISPLAYED_GEOMETRY_SPOOL_MAGIC.len()];
    file.read_exact(&mut magic)
        .map_err(|error| format!("read displayed geometry spool header: {error}"))?;
    if &magic != DISPLAYED_GEOMETRY_SPOOL_MAGIC {
        return Err("displayed geometry spool header did not match".to_string());
    }

    for _ in 0..spool.chunk_count {
        let mut len_bytes = [0u8; 8];
        file.read_exact(&mut len_bytes)
            .map_err(|error| format!("read displayed geometry chunk length: {error}"))?;
        let len = u64::from_le_bytes(len_bytes) as usize;
        if len > MAX_WIRE_PAYLOAD_LEN {
            return Err("displayed geometry chunk exceeds ABI limit".to_string());
        }
        let mut bytes = vec![0u8; len];
        file.read_exact(&mut bytes)
            .map_err(|error| format!("read displayed geometry chunk: {error}"))?;
        let chunk: WireDisplayedGeometryChunk = decode(&bytes)?;
        visitor(displayed_geometry_chunk_from_wire(chunk))?;
    }

    Ok(())
}

/// Serialize a wire value to MessagePack bytes.
///
/// # Errors
///
/// Returns a string if MessagePack serialization fails.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec(value).map_err(|error| error.to_string())
}

/// Deserialize a wire value from MessagePack bytes.
///
/// # Errors
///
/// Returns a string when the payload is too large or malformed.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    if bytes.len() > MAX_WIRE_PAYLOAD_LEN {
        return Err("wire payload exceeds ABI limit".to_string());
    }
    rmp_serde::from_slice(bytes).map_err(|error| error.to_string())
}

/// Encode a session to MessagePack bytes.
///
/// # Errors
///
/// Returns a string if session serialization fails.
pub fn encode_session(session: &Session) -> Result<Vec<u8>, String> {
    encode(session)
}

/// Decode a session from MessagePack bytes.
///
/// # Errors
///
/// Returns a string if the payload is too large or malformed.
pub fn decode_session(bytes: &[u8]) -> Result<Session, String> {
    decode(bytes)
}

/// Build a dynamic setting registry from wire entries.
///
/// # Errors
///
/// Returns a string if a descriptor collides with an existing setting.
pub fn dynamic_registry_from_wire(
    entries: &[WireDynamicSetting],
) -> Result<DynamicSettingRegistry, String> {
    let mut registry = DynamicSettingRegistry::new();
    for entry in entries {
        let descriptor: DynamicSettingDescriptor = entry.descriptor.clone().into();
        let mut store = DynamicSettingStore::new();
        if let Some(value) = &entry.value {
            store.set(&descriptor.name, value.clone());
        }
        registry.register(descriptor, Arc::new(RwLock::new(store)))?;
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_scene::{
        GpuBatchCommand, GpuBindingType, GpuCacheStats, GpuCacheStatus, GpuCachedHandle, GpuColor,
        GpuColorLoadOp, GpuColorOperations, GpuColorTargetState, GpuColorWriteMask,
        GpuCompareFunction, GpuDepthLoadOp, GpuDepthOperations, GpuDepthStencilState,
        GpuDeviceLimits, GpuExtent3d, GpuFace, GpuFragmentState, GpuFrontFace, GpuHandleKind,
        GpuMultisampleState, GpuOrigin3d, GpuPrimitiveState, GpuPrimitiveTopology,
        GpuRenderPassCommand, GpuRenderPassDescriptor, GpuRenderPipelineDescriptor,
        GpuStorageTextureAccess, GpuStoreOp, GpuTexelCopyBufferLayout, GpuTexelCopyTextureInfo,
        GpuTextureAspect, GpuTextureFormat, GpuTextureSampleType, GpuTextureViewDimension,
        GpuVertexAttribute, GpuVertexBufferLayout, GpuVertexFormat, GpuVertexState,
        GpuVertexStepMode, RenderArtifactBufferDescriptor, RenderArtifactBufferRole,
        RenderArtifactPrimitiveTopology, RenderArtifactRepDescriptor, RenderArtifactRepKind,
    };

    #[test]
    fn render_artifact_snapshot_value_roundtrips() {
        let handle = GpuHandle {
            id: 7,
            kind: GpuHandleKind::Buffer,
            generation: 1,
        };
        let snapshot = RenderArtifactSnapshotDescriptor {
            snapshot_id: 11,
            layout_version: 1,
            scene_generation: 42,
            scene_bounds_min: [-1.0, -2.0, -3.0],
            scene_bounds_max: [1.0, 2.0, 3.0],
            cull_pass_initialized: true,
            device_limits: GpuDeviceLimits {
                max_buffer_size: 1024,
                max_storage_buffer_binding_size: 512,
                max_compute_workgroups_per_dimension: 65_535,
                max_compute_invocations_per_workgroup: 256,
                max_compute_workgroup_size_x: 256,
                max_compute_workgroup_size_y: 256,
                max_compute_workgroup_size_z: 64,
                buffer_binding_array: true,
                storage_resource_binding_array: true,
            },
            buffers: vec![RenderArtifactBufferDescriptor {
                handle,
                role: RenderArtifactBufferRole::StdVertices,
                size: 240,
                stride: 24,
                element_count: 10,
            }],
            reps: vec![RenderArtifactRepDescriptor {
                object_id: 3,
                rep_kind: RenderArtifactRepKind::Cartoon,
                topology: RenderArtifactPrimitiveTopology::TriangleList,
                geometry: handle,
                count: None,
                indirect: None,
                element_count: 10,
                max_element_count: 10,
                atom_offset: 20,
                atom_count: 5,
                material_rgba: [0.1, 0.2, 0.3, 1.0],
                transparency: 0.25,
            }],
        };

        let value = WireCommandRuntimeValue::RenderArtifactSnapshotOpened(snapshot);
        let decoded: WireCommandRuntimeValue = decode(&encode(&value).unwrap()).unwrap();

        match decoded {
            WireCommandRuntimeValue::RenderArtifactSnapshotOpened(snapshot) => {
                assert_eq!(snapshot.snapshot_id, 11);
                assert_eq!(snapshot.buffers[0].handle, handle);
                assert_eq!(snapshot.reps[0].rep_kind, RenderArtifactRepKind::Cartoon);
                assert_eq!(snapshot.scene_bounds_min, [-1.0, -2.0, -3.0]);
                assert_eq!(snapshot.reps[0].atom_offset, 20);
                assert_eq!(snapshot.reps[0].transparency, 0.25);
            }
            _ => panic!("unexpected runtime value"),
        }
    }

    #[test]
    fn gpu_cached_values_roundtrip() {
        let handle = GpuHandle {
            id: 99,
            kind: GpuHandleKind::ComputePipeline,
            generation: 7,
        };
        let cached = WireCommandRuntimeValue::GpuCachedHandle(GpuCachedHandle {
            handle,
            status: GpuCacheStatus::Hit,
        });
        let decoded: WireCommandRuntimeValue = decode(&encode(&cached).unwrap()).unwrap();

        match decoded {
            WireCommandRuntimeValue::GpuCachedHandle(cached) => {
                assert_eq!(cached.handle, handle);
                assert_eq!(cached.status, GpuCacheStatus::Hit);
            }
            _ => panic!("unexpected cached handle value"),
        }

        let stats = WireCommandRuntimeValue::GpuCacheStats(GpuCacheStats {
            hits: 3,
            misses: 2,
            entries: 4,
        });
        let decoded: WireCommandRuntimeValue = decode(&encode(&stats).unwrap()).unwrap();

        match decoded {
            WireCommandRuntimeValue::GpuCacheStats(stats) => {
                assert_eq!(stats.hits, 3);
                assert_eq!(stats.misses, 2);
                assert_eq!(stats.entries, 4);
            }
            _ => panic!("unexpected cache stats value"),
        }
    }

    #[test]
    fn gpu_texture_batch_and_binding_roundtrip() {
        let texture = GpuHandle {
            id: 3,
            kind: GpuHandleKind::Texture,
            generation: 1,
        };
        let buffer = GpuHandle {
            id: 4,
            kind: GpuHandleKind::Buffer,
            generation: 1,
        };
        let command = GpuBatchCommand::CopyTextureToBuffer {
            source: GpuTexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: GpuOrigin3d { x: 0, y: 0, z: 0 },
                aspect: GpuTextureAspect::All,
            },
            destination: buffer,
            destination_layout: GpuTexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(4),
            },
            size: GpuExtent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
        };
        let decoded: GpuBatchCommand = decode(&encode(&command).unwrap()).unwrap();
        assert_eq!(decoded, command);

        let command = GpuBatchCommand::DispatchComputeIndirect {
            pipeline: GpuHandle {
                id: 8,
                kind: GpuHandleKind::ComputePipeline,
                generation: 1,
            },
            bind_groups: vec![GpuHandle {
                id: 9,
                kind: GpuHandleKind::BindGroup,
                generation: 1,
            }],
            indirect_buffer: buffer,
            indirect_offset: 12,
        };
        let decoded: GpuBatchCommand = decode(&encode(&command).unwrap()).unwrap();
        assert_eq!(decoded, command);

        let command = GpuBatchCommand::SetProfileScope {
            name: "surface_compact".to_string(),
        };
        let decoded: GpuBatchCommand = decode(&encode(&command).unwrap()).unwrap();
        assert_eq!(decoded, command);

        let binding = GpuBindingType::StorageTexture {
            access: GpuStorageTextureAccess::WriteOnly,
            format: GpuTextureFormat::Rgba8Unorm,
            view_dimension: GpuTextureViewDimension::D2,
        };
        let decoded: GpuBindingType = decode(&encode(&binding).unwrap()).unwrap();
        assert_eq!(decoded, binding);

        let sampled = GpuBindingType::Texture {
            sample_type: GpuTextureSampleType::Float { filterable: true },
            view_dimension: GpuTextureViewDimension::D2,
            multisampled: false,
        };
        let decoded: GpuBindingType = decode(&encode(&sampled).unwrap()).unwrap();
        assert_eq!(decoded, sampled);
    }

    #[test]
    fn gpu_render_pipeline_and_pass_roundtrip() {
        let shader = GpuHandle {
            id: 10,
            kind: GpuHandleKind::ShaderModule,
            generation: 1,
        };
        let layout = GpuHandle {
            id: 11,
            kind: GpuHandleKind::PipelineLayout,
            generation: 1,
        };
        let texture_view = GpuHandle {
            id: 12,
            kind: GpuHandleKind::TextureView,
            generation: 1,
        };
        let pipeline = GpuHandle {
            id: 13,
            kind: GpuHandleKind::RenderPipeline,
            generation: 1,
        };
        let descriptor = GpuRenderPipelineDescriptor {
            label: Some("wire.render".to_string()),
            layout,
            vertex: GpuVertexState {
                module: shader,
                entry_point: "vs_main".to_string(),
                buffers: vec![GpuVertexBufferLayout {
                    array_stride: 16,
                    step_mode: GpuVertexStepMode::Vertex,
                    attributes: vec![GpuVertexAttribute {
                        format: GpuVertexFormat::Float32x4,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
            },
            primitive: GpuPrimitiveState {
                topology: GpuPrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: GpuFrontFace::Ccw,
                cull_mode: Some(GpuFace::Back),
            },
            depth_stencil: Some(GpuDepthStencilState {
                format: GpuTextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: GpuCompareFunction::LessEqual,
            }),
            multisample: GpuMultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(GpuFragmentState {
                module: shader,
                entry_point: "fs_main".to_string(),
                targets: vec![Some(GpuColorTargetState {
                    format: GpuTextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: GpuColorWriteMask::ALL,
                })],
            }),
        };

        let request = WireCommandRuntimeRequest::GpuCreateCachedRenderPipeline {
            id: 77,
            descriptor: descriptor.clone(),
        };
        let decoded: WireCommandRuntimeRequest = decode(&encode(&request).unwrap()).unwrap();
        match decoded {
            WireCommandRuntimeRequest::GpuCreateCachedRenderPipeline {
                id,
                descriptor: got,
            } => {
                assert_eq!(id, 77);
                assert_eq!(got, descriptor);
            }
            _ => panic!("unexpected render pipeline request"),
        }

        let command = GpuBatchCommand::RenderPass {
            descriptor: GpuRenderPassDescriptor {
                label: Some("wire.pass".to_string()),
                color_attachments: vec![Some(patinae_scene::GpuRenderPassColorAttachment {
                    view: texture_view,
                    ops: GpuColorOperations {
                        load: GpuColorLoadOp::Clear(GpuColor {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: GpuStoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(
                    patinae_scene::GpuRenderPassDepthStencilAttachment {
                        view: texture_view,
                        depth_ops: Some(GpuDepthOperations {
                            load: GpuDepthLoadOp::Clear(1.0),
                            store: GpuStoreOp::Store,
                        }),
                    },
                ),
            },
            commands: vec![
                GpuRenderPassCommand::SetPipeline { pipeline },
                GpuRenderPassCommand::Draw {
                    vertex_start: 0,
                    vertex_count: 3,
                    instance_start: 0,
                    instance_count: 1,
                },
            ],
        };
        let decoded: GpuBatchCommand = decode(&encode(&command).unwrap()).unwrap();
        assert_eq!(decoded, command);
    }
}
