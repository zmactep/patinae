//! Viewer abstraction trait
//!
//! This module defines the `ViewerLike` trait that abstracts the viewer interface,
//! allowing commands to work with different viewer implementations (e.g., `Viewer`,
//! GUI adapters).
//!
//! The trait uses accessor methods (`movie()`, `scenes()`, `selections()`, etc.)
//! to provide default implementations for ~50 methods, so implementors only need
//! to provide the accessors plus a few GPU/window-specific overrides.

use std::path::Path;

use patinae_color::NamedPalette;
use patinae_select::SelectionResult;
use patinae_settings::Settings;
use serde::{Deserialize, Serialize};

use crate::camera::Camera;
use crate::error::SceneResult;
use crate::movie::{LoopMode, Movie};
use crate::object::{MeasurementResolveOptions, Object, ObjectRegistry};
use crate::scene::SceneManager;
use crate::selection::SelectionManager;
use crate::session::Session;
use crate::view::ViewManager;

/// Image for display as a viewport overlay
///
/// Used by the `ray` command to show raytraced output, but any source
/// (plugins, computed images, etc.) can provide an image for display.
/// The overlay persists until camera or scene changes occur.
#[derive(Clone, Serialize, Deserialize)]
pub struct ViewportImage {
    /// RGBA image data, row-major, top-to-bottom
    pub data: Vec<u8>,
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
}

/// Trait for types that can serve as a viewer backend for command execution
///
/// This trait abstracts the viewer interface, allowing commands to work with
/// different viewer implementations (e.g., `Viewer`, GUI adapters).
///
/// # Required methods
///
/// Implementors must provide accessor methods for the core state managers
/// and a few GPU/window-specific operations. Most command-facing methods
/// have default implementations that use these accessors.
pub trait ViewerLike {
    // =========================================================================
    // Required Accessors — Core State
    // =========================================================================

    /// Get a reference to the object registry
    fn objects(&self) -> &ObjectRegistry;

    /// Get a mutable reference to the object registry
    fn objects_mut(&mut self) -> &mut ObjectRegistry;

    /// Get a reference to the camera
    fn camera(&self) -> &Camera;

    /// Get a mutable reference to the camera
    fn camera_mut(&mut self) -> &mut Camera;

    /// Get a reference to the global settings
    fn settings(&self) -> &Settings;

    /// Get a mutable reference to the global settings
    fn settings_mut(&mut self) -> &mut Settings;

    /// Request a redraw
    fn request_redraw(&mut self);

    /// Get a reference to the full session
    fn session(&self) -> &Session;

    /// Get a mutable reference to the full session
    fn session_mut(&mut self) -> &mut Session;

    /// Reconcile durable recent atom paths against current scene state.
    fn reconcile_recent_atoms(&mut self) {
        self.session_mut().reconcile_recent_atoms();
    }

    /// Inserts an object through the shared session mutation seam.
    fn insert_object(&mut self, object: Box<dyn Object>) {
        self.session_mut().insert_object(object);
    }

    /// Inserts objects sequentially and reconciles recent paths once.
    fn insert_objects(&mut self, objects: Vec<Box<dyn Object>>) {
        self.session_mut().insert_objects(objects);
    }

    /// Rename an object through the shared session mutation seam.
    ///
    /// # Errors
    ///
    /// Returns an error when the registry rejects the rename.
    fn rename_object(&mut self, old_name: &str, new_name: &str) -> SceneResult<()> {
        self.session_mut().rename_object(old_name, new_name)
    }

    /// Remove an object through the shared session mutation seam.
    fn remove_object(&mut self, name: &str) -> bool {
        self.session_mut().remove_object(name)
    }

    /// Remove molecule atoms through the shared session mutation seam.
    ///
    /// # Errors
    ///
    /// Returns an error when `source_name` is not a molecule object.
    fn remove_molecule_atoms(
        &mut self,
        source_name: &str,
        indices: &[patinae_mol::AtomIndex],
    ) -> SceneResult<usize> {
        self.session_mut()
            .remove_molecule_atoms(source_name, indices)
    }

    /// Clear all objects through the shared session mutation seam.
    fn clear_objects(&mut self) {
        self.session_mut().clear_objects();
    }

    /// Apply immediate recent-atom effects for one changed global setting.
    fn reconcile_recent_atom_setting(&mut self, setting_name: &str) {
        self.session_mut()
            .reconcile_recent_atom_setting(setting_name);
    }

    /// Replace the entire session (for loading .pse/.prs files)
    ///
    /// Implementors should mark all objects as dirty so representations rebuild.
    fn replace_session(&mut self, session: Session);

    // =========================================================================
    // Required Accessors — Sub-managers
    // =========================================================================

    /// Get a reference to the movie player
    fn movie(&self) -> &Movie;

    /// Get a mutable reference to the movie player
    fn movie_mut(&mut self) -> &mut Movie;

    /// Get a reference to the scene manager
    fn scenes(&self) -> &SceneManager;

    /// Get a mutable reference to the scene manager
    fn scenes_mut(&mut self) -> &mut SceneManager;

    /// Get a reference to the view manager
    fn views(&self) -> &ViewManager;

    /// Get a mutable reference to the view manager
    fn views_mut(&mut self) -> &mut ViewManager;

    /// Get a reference to the selection manager
    fn selections(&self) -> &SelectionManager;

    /// Get a mutable reference to the selection manager
    fn selections_mut(&mut self) -> &mut SelectionManager;

    /// Get a reference to the named colors table
    fn named_palette(&self) -> &NamedPalette;
    /// Get a mutable reference to the named colors table
    fn named_palette_mut(&mut self) -> &mut NamedPalette;

    // =========================================================================
    // Required Accessors — Simple State
    // =========================================================================

    /// Get the clear (background) color
    fn clear_color(&self) -> [f32; 3];

    /// Set the clear (background) color
    fn set_clear_color(&mut self, color: [f32; 3]);

    /// Get a reference to the stored viewport image overlay, if any
    fn viewport_image_ref(&self) -> Option<&ViewportImage>;

    /// Set or clear the stored viewport image overlay
    fn set_viewport_image_internal(&mut self, image: Option<ViewportImage>);

    // =========================================================================
    // Camera / View — Default Implementations
    // =========================================================================

    /// Zoom to fit all objects while preserving rotation
    fn zoom_all(&mut self, buffer: f32) {
        let options = MeasurementResolveOptions::from_settings(&self.settings().measurement);
        if let Some((min, max)) = self.objects().extent_with_options(options) {
            self.camera_mut().zoom_to(min, max, buffer);
            self.set_viewport_image_internal(None);
            self.request_redraw();
        }
    }

    /// Zoom to fit a specific object while preserving rotation
    ///
    /// For groups, uses the combined extent of all children.
    fn zoom_on(&mut self, name: &str, buffer: f32) {
        let options = MeasurementResolveOptions::from_settings(&self.settings().measurement);
        let extent = if self.objects().get_group(name).is_some() {
            self.objects().group_extent_with_options(name, options)
        } else {
            self.objects().object_extent_with_options(name, options)
        };
        if let Some((min, max)) = extent {
            self.camera_mut().zoom_to(min, max, buffer);
            self.set_viewport_image_internal(None);
            self.request_redraw();
        }
    }

    /// Center on all objects without changing zoom or rotation
    fn center_all(&mut self) {
        let options = MeasurementResolveOptions::from_settings(&self.settings().measurement);
        if let Some((min, max)) = self.objects().extent_with_options(options) {
            self.camera_mut().center_to(min, max);
            self.set_viewport_image_internal(None);
            self.request_redraw();
        }
    }

    /// Center on a specific object without changing zoom or rotation
    ///
    /// For groups, uses the combined extent of all children.
    fn center_on(&mut self, name: &str) {
        let options = MeasurementResolveOptions::from_settings(&self.settings().measurement);
        let extent = if self.objects().get_group(name).is_some() {
            self.objects().group_extent_with_options(name, options)
        } else {
            self.objects().object_extent_with_options(name, options)
        };
        if let Some((min, max)) = extent {
            self.camera_mut().center_to(min, max);
            self.set_viewport_image_internal(None);
            self.request_redraw();
        }
    }

    /// Reset the camera to default view
    fn reset_view(&mut self) {
        *self.camera_mut() = Camera::new();
        self.set_viewport_image_internal(None);
        let options = MeasurementResolveOptions::from_settings(&self.settings().measurement);
        if let Some((min, max)) = self.objects().extent_with_options(options) {
            self.camera_mut().reset_view(min, max);
            self.request_redraw();
        }
    }

    // =========================================================================
    // Color — Default Implementations
    // =========================================================================

    /// Get a color index by name
    fn color_index(&self, name: &str) -> Option<u32> {
        self.named_palette().get_by_name(name).map(|(idx, _)| idx)
    }

    /// Set the background color
    fn set_background_color(&mut self, r: f32, g: f32, b: f32) {
        self.set_clear_color([r, g, b]);
        self.request_redraw();
    }

    /// Reset the background color to the current theme.
    fn reset_background_color(&mut self) {
        let color = self.session().palette.viewport_bg.to_array();
        {
            let session = self.session_mut();
            session.clear_color = color;
            session.clear_color_set = false;
        }
        self.request_redraw();
    }

    // =========================================================================
    // Named Selections — Default Implementations
    // =========================================================================

    /// Get a named selection expression by name
    fn get_selection(&self, name: &str) -> Option<&str> {
        self.selections().get_expression(name)
    }

    /// Define (store) a named selection expression.
    /// Cache will be lazily populated on the next render.
    fn define_selection(&mut self, name: &str, selection: &str) {
        self.selections_mut().define(name, selection);
        self.request_redraw();
    }

    /// Define a named selection with pre-computed evaluation results.
    fn define_selection_with_results(
        &mut self,
        name: &str,
        selection: &str,
        results: Vec<(String, SelectionResult)>,
    ) {
        self.selections_mut()
            .define_with_results(name, selection, results);
        self.request_redraw();
    }

    /// Remove a named selection, returns true if it existed
    fn remove_selection(&mut self, name: &str) -> bool {
        let removed = self.selections_mut().remove(name);
        if removed {
            self.request_redraw();
        }
        removed
    }

    /// Rename a named selection, returns true if it existed and was renamed
    fn rename_selection(&mut self, old_name: &str, new_name: &str) -> bool {
        let renamed = self.selections_mut().rename(old_name, new_name);
        if renamed {
            self.request_redraw();
        }
        renamed
    }

    /// Get all named selection names
    fn selection_names(&self) -> Vec<String> {
        self.selections().names()
    }

    /// Set the visibility of a named selection's indicators
    fn set_selection_visible(&mut self, name: &str, visible: bool) {
        self.selections_mut().set_visible(name, visible);
        self.request_redraw();
    }

    /// Check if a named selection's indicators are visible
    fn is_selection_visible(&self, name: &str) -> bool {
        self.selections().is_visible(name)
    }

    /// Set the indicated selection (shows pink indicators in the 3D view)
    fn indicate_selection(&mut self, selection: &str) {
        self.selections_mut().indicate(selection);
        self.request_redraw();
    }

    /// Clear the indicated selection (hides all selection indicators)
    fn clear_indication(&mut self) {
        self.selections_mut().clear_indication();
        self.request_redraw();
    }

    /// Get the currently indicated selection expression
    fn indicated_selection(&self) -> Option<&str> {
        self.selections().indicated_selection()
    }

    // =========================================================================
    // Screenshot Capture — Optional Overrides
    // =========================================================================

    /// Capture a PNG screenshot (optional - not all viewers support this)
    fn capture_png(
        &mut self,
        _path: &Path,
        _width: Option<u32>,
        _height: Option<u32>,
    ) -> Result<(), String> {
        Err("Screenshot capture not supported by this viewer".to_string())
    }

    /// Display a command-scoped GPU RGBA8 buffer as the viewport image.
    fn set_viewport_gpu_image_from_buffer(
        &mut self,
        _buffer: crate::GpuHandle,
        _width: u32,
        _height: u32,
    ) -> Result<(), String> {
        Err("GPU viewport images are not supported by this viewer".to_string())
    }

    /// Display a native GPU RGBA8 buffer as the viewport image.
    fn set_viewport_gpu_image_from_wgpu_buffer(
        &mut self,
        _buffer: &wgpu::Buffer,
        _buffer_size: u64,
        _width: u32,
        _height: u32,
    ) -> Result<(), String> {
        Err("GPU viewport images are not supported by this viewer".to_string())
    }

    /// Save the current GPU viewport image as PNG, if one exists.
    fn save_viewport_gpu_image(&mut self, _path: &Path) -> Result<Option<(u32, u32)>, String> {
        Ok(None)
    }

    /// Clear any transient GPU viewport image.
    fn clear_viewport_gpu_image(&mut self) {}

    /// Export the current displayed scene as renderer-neutral geometry.
    #[cfg(feature = "render-bridge")]
    fn export_displayed_geometry(
        &mut self,
        _options: &patinae_render::GeometryExportOptions,
    ) -> Result<patinae_render::DisplayedGeometry, String> {
        Err("Displayed geometry export not supported by this viewer".to_string())
    }

    /// Visit displayed geometry in one or more renderer-neutral chunks.
    #[cfg(feature = "render-bridge")]
    fn for_each_displayed_geometry_chunk(
        &mut self,
        options: &patinae_render::GeometryExportOptions,
        visitor: &mut dyn FnMut(patinae_render::DisplayedGeometry) -> Result<(), String>,
    ) -> Result<(), String> {
        visitor(self.export_displayed_geometry(options)?)
    }

    /// Visit compact trace geometry in one or more runtime chunks.
    #[cfg(feature = "render-bridge")]
    fn for_each_trace_geometry_chunk(
        &mut self,
        options: &patinae_render::GeometryExportOptions,
        visitor: &mut dyn FnMut(patinae_render::TraceGeometryChunk) -> Result<(), String>,
    ) -> Result<(), String> {
        self.for_each_displayed_geometry_chunk(options, &mut |displayed| {
            visitor(patinae_render::TraceGeometryChunk::from_displayed(
                &displayed,
            ))
        })
    }

    /// Visit renderer-owned GPU artifacts for the currently displayed scene.
    #[cfg(feature = "render-bridge")]
    fn visit_render_artifacts(
        &mut self,
        _visitor: &mut dyn FnMut(patinae_render::RenderArtifactSnapshot<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        Err("render artifact snapshot was not supplied by the host".to_string())
    }

    /// Open a command-scoped renderer artifact snapshot.
    fn open_render_artifact_snapshot(
        &mut self,
    ) -> Result<crate::RenderArtifactSnapshotDescriptor, String> {
        Err("render artifact snapshot runtime is not available".to_string())
    }

    /// Close a command-scoped renderer artifact snapshot.
    fn close_render_artifact_snapshot(&mut self, _snapshot_id: u64) -> Result<(), String> {
        Err("render artifact snapshot runtime is not available".to_string())
    }

    /// Query GPU device limits for command-scoped plugin compute work.
    fn gpu_device_limits_for_plugins(&mut self) -> Result<crate::GpuDeviceLimits, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create a host-owned GPU buffer.
    fn gpu_create_buffer(
        &mut self,
        _descriptor: crate::GpuBufferDescriptor,
        _initial_data: Option<Vec<u8>>,
    ) -> Result<crate::GpuHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create a host-owned GPU texture.
    fn gpu_create_texture(
        &mut self,
        _descriptor: crate::GpuTextureDescriptor,
    ) -> Result<crate::GpuHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create a host-owned GPU texture view.
    fn gpu_create_texture_view(
        &mut self,
        _descriptor: crate::GpuTextureViewDescriptor,
    ) -> Result<crate::GpuHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create a host-owned GPU sampler.
    fn gpu_create_sampler(
        &mut self,
        _descriptor: crate::GpuSamplerDescriptor,
    ) -> Result<crate::GpuHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Write bytes into a host-owned GPU buffer.
    fn gpu_write_buffer(
        &mut self,
        _buffer: crate::GpuHandle,
        _offset: u64,
        _data: Vec<u8>,
    ) -> Result<(), String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Copy bytes between host-owned GPU buffers.
    fn gpu_copy_buffer_to_buffer(
        &mut self,
        _source: crate::GpuHandle,
        _source_offset: u64,
        _destination: crate::GpuHandle,
        _destination_offset: u64,
        _size: u64,
    ) -> Result<(), String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Read bytes from a host-owned GPU buffer.
    fn gpu_read_buffer(
        &mut self,
        _buffer: crate::GpuHandle,
        _offset: u64,
        _size: u64,
    ) -> Result<Vec<u8>, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create a WGSL shader module.
    fn gpu_create_shader_module(
        &mut self,
        _descriptor: crate::GpuShaderModuleDescriptor,
    ) -> Result<crate::GpuHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create a bind-group layout.
    fn gpu_create_bind_group_layout(
        &mut self,
        _descriptor: crate::GpuBindGroupLayoutDescriptor,
    ) -> Result<crate::GpuHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create a pipeline layout.
    fn gpu_create_pipeline_layout(
        &mut self,
        _descriptor: crate::GpuPipelineLayoutDescriptor,
    ) -> Result<crate::GpuHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create a compute pipeline.
    fn gpu_create_compute_pipeline(
        &mut self,
        _descriptor: crate::GpuComputePipelineDescriptor,
    ) -> Result<crate::GpuHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create a render pipeline.
    fn gpu_create_render_pipeline(
        &mut self,
        _descriptor: crate::GpuRenderPipelineDescriptor,
    ) -> Result<crate::GpuHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create or lease a cached WGSL shader module.
    fn gpu_create_cached_shader_module(
        &mut self,
        _descriptor: crate::GpuShaderModuleDescriptor,
    ) -> Result<crate::GpuCachedHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create or lease a cached bind-group layout.
    fn gpu_create_cached_bind_group_layout(
        &mut self,
        _descriptor: crate::GpuBindGroupLayoutDescriptor,
    ) -> Result<crate::GpuCachedHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create or lease a cached pipeline layout.
    fn gpu_create_cached_pipeline_layout(
        &mut self,
        _descriptor: crate::GpuPipelineLayoutDescriptor,
    ) -> Result<crate::GpuCachedHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create or lease a cached compute pipeline.
    fn gpu_create_cached_compute_pipeline(
        &mut self,
        _descriptor: crate::GpuComputePipelineDescriptor,
    ) -> Result<crate::GpuCachedHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create or lease a cached render pipeline.
    fn gpu_create_cached_render_pipeline(
        &mut self,
        _descriptor: crate::GpuRenderPipelineDescriptor,
    ) -> Result<crate::GpuCachedHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Query persistent GPU cache counters for the active plugin.
    fn gpu_cache_stats(&mut self) -> Result<crate::GpuCacheStats, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Drop persistent cached GPU resources for the active plugin.
    fn gpu_drop_plugin_cache(&mut self) -> Result<(), String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Create a bind group.
    fn gpu_create_bind_group(
        &mut self,
        _descriptor: crate::GpuBindGroupDescriptor,
    ) -> Result<crate::GpuHandle, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Dispatch a compute pipeline.
    fn gpu_dispatch_compute(
        &mut self,
        _pipeline: crate::GpuHandle,
        _bind_groups: Vec<crate::GpuHandle>,
        _workgroups: [u32; 3],
    ) -> Result<(), String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Submit one ordered GPU command batch.
    fn gpu_submit_batch(
        &mut self,
        _batch: crate::GpuSubmitBatch,
    ) -> Result<crate::GpuBatchResult, String> {
        Err("GPU command runtime is not available".to_string())
    }

    /// Drop command-scoped GPU handles before command end.
    fn gpu_drop_handles(&mut self, _handles: Vec<crate::GpuHandle>) -> Result<(), String> {
        Err("GPU command runtime is not available".to_string())
    }

    // =========================================================================
    // GPU Access — Optional Overrides
    // =========================================================================

    /// Get the wgpu device (if available)
    fn gpu_device(&self) -> Option<&wgpu::Device> {
        None
    }

    /// Get the wgpu queue (if available)
    fn gpu_queue(&self) -> Option<&wgpu::Queue> {
        None
    }

    /// Ensure all representations are built and up-to-date
    fn prepare_render(&mut self) {}

    // =========================================================================
    // Viewport Image Overlay — Default Implementations
    // =========================================================================

    /// Store an image for display as a viewport overlay
    fn set_viewport_image(&mut self, image: Option<ViewportImage>) {
        self.set_viewport_image_internal(image);
        self.request_redraw();
    }

    /// Get the stored viewport image overlay, if any
    fn get_viewport_image(&self) -> Option<&ViewportImage> {
        self.viewport_image_ref()
    }

    /// Clear the viewport image overlay
    fn clear_viewport_image(&mut self) {
        self.set_viewport_image_internal(None);
    }

    // =========================================================================
    // Async Operations — Optional Override
    // =========================================================================

    /// Request an async fetch operation from RCSB PDB (non-blocking)
    ///
    /// Returns `true` if the request was accepted, `false` if async fetch
    /// is not supported (caller should use sync fallback).
    fn request_async_fetch(&mut self, _code: &str, _name: &str, _format: u8) -> bool {
        false
    }

    // =========================================================================
    // Scene Management
    //
    // scene_store and scene_recall MUST be per-impl because they need
    // simultaneous borrows on different parts of self (e.g. &mut scenes + &camera).
    // The simpler methods have default implementations.
    // =========================================================================

    /// Store the current view as a named scene
    fn scene_store(&mut self, _key: &str, _storemask: u32);

    /// Recall a named scene
    fn scene_recall(&mut self, _key: &str, _animate: bool, _duration: f32) -> Result<(), String>;

    /// Delete a named scene
    fn scene_delete(&mut self, key: &str) -> bool {
        self.scenes_mut().delete(key).is_some()
    }

    /// Get list of all scene names
    fn scene_list(&self) -> Vec<String> {
        self.scenes().list().to_vec()
    }

    /// Rename a scene
    fn scene_rename(&mut self, old_key: &str, new_key: &str) -> Result<(), String> {
        self.scenes_mut()
            .rename(old_key, new_key)
            .map_err(|e| e.to_string())
    }

    /// Clear all scenes
    fn scene_clear(&mut self) {
        self.scenes_mut().clear();
    }

    // =========================================================================
    // Movie Control — Default Implementations
    // =========================================================================

    /// Start movie playback
    fn movie_play(&mut self) {
        let loop_setting = self.settings().movie.movie_loop;
        let fps = self.settings().movie.movie_fps;
        self.movie_mut().set_loop_mode(if loop_setting {
            LoopMode::Loop
        } else {
            LoopMode::Once
        });
        self.movie_mut().set_fps(fps);
        self.movie_mut().play();
        self.request_redraw();
    }

    /// Stop movie and reset to first frame
    fn movie_stop(&mut self) {
        self.movie_mut().stop();
        self.sync_movie_to_objects();
        self.request_redraw();
    }

    /// Pause movie playback
    fn movie_pause(&mut self) {
        self.movie_mut().pause();
        self.request_redraw();
    }

    /// Toggle play/pause
    fn movie_toggle(&mut self) {
        self.movie_mut().toggle();
        self.request_redraw();
    }

    /// Go to a specific frame (0-indexed)
    fn movie_goto(&mut self, frame: usize) {
        self.movie_mut().goto_frame(frame);
        self.sync_movie_to_objects();
        self.request_redraw();
    }

    /// Advance to next frame
    fn movie_next(&mut self) {
        self.movie_mut().next_frame();
        self.sync_movie_to_objects();
        self.request_redraw();
    }

    /// Go to previous frame
    fn movie_prev(&mut self) {
        self.movie_mut().prev_frame();
        self.sync_movie_to_objects();
        self.request_redraw();
    }

    /// Update `Movie::n_object_states` from the current max across all objects.
    /// Call after loading/deleting objects.
    fn update_movie_state_count(&mut self) {
        self.session_mut().refresh_movie_state_count();
    }

    /// Sync the movie's current frame to all objects' display states and camera.
    fn sync_movie_to_objects(&mut self) {
        self.session_mut().sync_movie_frame();
    }

    /// Set frames per second
    fn movie_set_fps(&mut self, fps: f32) {
        self.movie_mut().set_fps(fps);
    }

    /// Get total number of frames (effective: movie frames or max object states)
    fn movie_frame_count(&self) -> usize {
        self.movie().effective_frame_count()
    }

    /// Get current frame index (0-indexed)
    fn movie_current_frame(&self) -> usize {
        self.movie().current_frame()
    }

    /// Check if movie is currently playing
    fn movie_is_playing(&self) -> bool {
        self.movie().is_playing()
    }

    /// Set loop mode: 0 = once, 1 = loop, 2 = swing
    fn movie_set_loop_mode(&mut self, mode: u8) {
        let loop_mode = match mode {
            1 => LoopMode::Loop,
            2 => LoopMode::Swing,
            _ => LoopMode::Once,
        };
        self.movie_mut().set_loop_mode(loop_mode);
    }

    /// Set number of movie frames
    fn movie_set_frame_count(&mut self, count: usize) {
        self.movie_mut().set_frame_count(count);
    }

    /// Set movie frames from a specification (state index per frame)
    fn movie_set_from_spec(&mut self, states: Vec<usize>) {
        self.movie_mut().set_from_spec(states);
    }

    /// Append movie frames from a specification
    fn movie_append_from_spec(&mut self, states: Vec<usize>) {
        self.movie_mut().append_from_spec(states);
    }

    /// Store a camera keyframe at the given frame (0-indexed)
    fn movie_store_view(&mut self, frame: usize) {
        let view = self.camera().current_view();
        self.movie_mut().store_camera_keyframe(frame, view);
        let loop_movie = self.settings().movie.movie_loop;
        if self.settings().movie.movie_auto_interpolate {
            self.movie_mut().interpolate_keyframes(loop_movie);
        }
        self.request_redraw();
    }

    /// Store a scene keyframe at the given frame (0-indexed)
    fn movie_store_scene(&mut self, frame: usize, scene_name: &str) {
        let view = self
            .scenes()
            .get(scene_name)
            .map(|s| s.view.clone())
            .unwrap_or_else(|| self.camera().current_view());
        self.movie_mut()
            .store_scene_keyframe(frame, scene_name, view);
        let loop_movie = self.settings().movie.movie_loop;
        if self.settings().movie.movie_auto_interpolate {
            self.movie_mut().interpolate_keyframes(loop_movie);
        }
        self.request_redraw();
    }

    /// Store an object keyframe at the given frame
    fn movie_store_object(&mut self, frame: usize, object: &str, state: Option<usize>) {
        // Capture the object's current TTT transform matrix
        let transform = self
            .objects()
            .get_molecule(object)
            .map(|mol| mol.state().transform.clone());
        self.movie_mut()
            .store_object_keyframe(frame, object, state, transform);
        // Auto-interpolate object transforms between keyframes
        let loop_movie = self.settings().movie.movie_loop;
        if self.settings().movie.movie_auto_interpolate {
            self.movie_mut().interpolate_object_keyframes(loop_movie);
        }
        self.request_redraw();
    }

    /// Clear camera keyframe(s). None = clear all.
    fn movie_clear_view(&mut self, frame: Option<usize>) {
        self.movie_mut().clear_camera_keyframes(frame);
    }

    /// Trigger keyframe interpolation
    fn movie_interpolate(&mut self) {
        let loop_movie = self.settings().movie.movie_loop;
        self.movie_mut().interpolate_keyframes(loop_movie);
        self.request_redraw();
    }

    /// Render a specific frame to PNG (for mpng)
    fn capture_frame_png(
        &mut self,
        _frame: usize,
        _path: &Path,
        _width: Option<u32>,
        _height: Option<u32>,
    ) -> Result<(), String> {
        Err("Not supported".into())
    }

    // =========================================================================
    // Rock Animation — Default Implementations
    // =========================================================================

    /// Toggle rock mode (Y-axis oscillation)
    fn rock_toggle(&mut self) {
        let current = self.movie().is_rock_enabled();
        self.movie_mut().set_rock(!current);
        self.request_redraw();
    }

    /// Set rock mode explicitly
    fn rock_set(&mut self, enabled: bool) {
        self.movie_mut().set_rock(enabled);
        self.request_redraw();
    }

    /// Check if rock is enabled
    fn rock_is_enabled(&self) -> bool {
        self.movie().is_rock_enabled()
    }

    // =========================================================================
    // Named View Storage — Default Implementations
    //
    // view_store uses camera().current_view() then views_mut() — no conflict.
    // view_recall MUST be per-impl (needs &views + &mut camera simultaneously).
    // =========================================================================

    /// Store the current camera view under a name
    fn view_store(&mut self, key: &str) {
        let view = self.camera().current_view();
        self.views_mut().store_view(key, view);
    }

    /// Recall a named view (MUST be per-impl due to borrow conflict)
    fn view_recall(&mut self, _key: &str, _animate: f32) -> Result<(), String>;

    /// Delete a named view
    fn view_delete(&mut self, key: &str) -> bool {
        self.views_mut().delete(key).is_some()
    }

    /// Get list of all stored view names
    fn view_list(&self) -> Vec<String> {
        self.views().list().to_vec()
    }

    /// Clear all stored views
    fn view_clear(&mut self) {
        self.views_mut().clear();
    }

    // =========================================================================
    // Viewport / Window Size — Optional Overrides
    // =========================================================================

    /// Get the current viewport size (width, height)
    fn viewport_size(&self) -> (u32, u32) {
        (0, 0)
    }

    /// Request a viewport resize
    fn viewport_set_size(&mut self, _width: u32, _height: u32) {
        // Default: no-op
    }

    // =========================================================================
    // Fullscreen Mode — Optional Overrides
    // =========================================================================

    /// Check if fullscreen mode is active
    fn is_fullscreen(&self) -> bool {
        false
    }

    /// Set fullscreen mode
    fn set_fullscreen(&mut self, _enabled: bool) {
        // Default: no-op
    }

    /// Toggle fullscreen mode
    fn toggle_fullscreen(&mut self) {
        // Default: no-op
    }
}
