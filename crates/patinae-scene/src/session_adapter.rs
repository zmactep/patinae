//! SessionAdapter — bridges Session to the ViewerLike trait
//!
//! [`SessionAdapter`] borrows a [`Session`] and an optional [`RenderContext`]
//! and implements [`ViewerLike`], allowing the command system to operate on
//! pure scene state with optional GPU support.

use std::path::Path;

use patinae_color::NamedPalette;
use patinae_settings::Settings;

use crate::camera::Camera;
use crate::movie::Movie;
use crate::object::ObjectRegistry;
use crate::render_target::CaptureRenderer;
use crate::scene::{SceneManager, SceneStoreMask};
use crate::selection::SelectionManager;
use crate::session::Session;
use crate::view::ViewManager;
use crate::viewer_trait::{ViewerLike, ViewportImage};

/// Callback type for async fetch operations (GUI sets this; headless leaves None).
type AsyncFetchFn<'a> = Box<dyn Fn(&str, &str, u8) -> bool + 'a>;

/// Adapter that wraps a [`Session`] to implement [`ViewerLike`] for command execution.
///
/// Borrows a mutable reference to a `Session` (scene state) plus an optional
/// `RenderContext` (GPU resources) and a `needs_redraw` flag.
pub struct SessionAdapter<'a> {
    /// The scene state
    pub session: &'a mut Session,
    /// Headless render target (None if no GPU host is attached / not yet
    /// initialized). Capture commands and the (few) commands that need
    /// raw GPU handles consult this trait object instead of a concrete
    /// renderer type.
    pub render_context: Option<&'a mut (dyn CaptureRenderer + 'a)>,
    /// Default viewport size for capture when width/height not specified
    pub default_size: (u32, u32),
    /// Redraw flag — set to true when a re-render is needed
    pub needs_redraw: &'a mut bool,
    /// Optional callback for async fetch (GUI sets this; headless leaves None)
    pub async_fetch_fn: Option<AsyncFetchFn<'a>>,
}

impl<'a> ViewerLike for SessionAdapter<'a> {
    // =========================================================================
    // Required Accessors
    // =========================================================================

    fn objects(&self) -> &ObjectRegistry {
        &self.session.registry
    }
    fn objects_mut(&mut self) -> &mut ObjectRegistry {
        &mut self.session.registry
    }
    fn camera(&self) -> &Camera {
        &self.session.camera
    }
    fn camera_mut(&mut self) -> &mut Camera {
        &mut self.session.camera
    }
    fn settings(&self) -> &Settings {
        &self.session.settings
    }
    fn settings_mut(&mut self) -> &mut Settings {
        &mut self.session.settings
    }
    fn movie(&self) -> &Movie {
        &self.session.movie
    }
    fn movie_mut(&mut self) -> &mut Movie {
        &mut self.session.movie
    }
    fn scenes(&self) -> &SceneManager {
        &self.session.scenes
    }
    fn scenes_mut(&mut self) -> &mut SceneManager {
        &mut self.session.scenes
    }
    fn views(&self) -> &ViewManager {
        &self.session.views
    }
    fn views_mut(&mut self) -> &mut ViewManager {
        &mut self.session.views
    }
    fn selections(&self) -> &SelectionManager {
        &self.session.selections
    }
    fn selections_mut(&mut self) -> &mut SelectionManager {
        &mut self.session.selections
    }
    fn named_palette(&self) -> &NamedPalette {
        &self.session.named_palette
    }
    fn named_palette_mut(&mut self) -> &mut NamedPalette {
        &mut self.session.named_palette
    }
    fn clear_color(&self) -> [f32; 3] {
        self.session.clear_color
    }
    fn set_clear_color(&mut self, color: [f32; 3]) {
        self.session.clear_color = color;
        self.session.clear_color_set = true;
    }
    fn viewport_image_ref(&self) -> Option<&ViewportImage> {
        self.session.viewport_image.as_ref()
    }
    fn set_viewport_image_internal(&mut self, image: Option<ViewportImage>) {
        self.session.viewport_image = image;
        if let Some(render_context) = self.render_context.as_deref_mut() {
            render_context.clear_viewport_gpu_image();
        }
    }

    fn request_redraw(&mut self) {
        *self.needs_redraw = true;
    }

    fn session(&self) -> &Session {
        self.session
    }
    fn session_mut(&mut self) -> &mut Session {
        self.session
    }

    fn replace_session(&mut self, session: Session) {
        self.session.replace_contents(session);
        *self.needs_redraw = true;
    }

    // =========================================================================
    // Async fetch
    // =========================================================================

    fn request_async_fetch(&mut self, code: &str, name: &str, format: u8) -> bool {
        if let Some(ref f) = self.async_fetch_fn {
            f(code, name, format)
        } else {
            false
        }
    }

    // =========================================================================
    // GPU-specific overrides
    // =========================================================================

    fn capture_png(
        &mut self,
        path: &Path,
        width: Option<u32>,
        height: Option<u32>,
    ) -> Result<(), String> {
        let target = self
            .render_context
            .as_deref_mut()
            .ok_or_else(|| "No render context available".to_string())?;
        let w = width.unwrap_or(self.default_size.0);
        let h = height.unwrap_or(self.default_size.1);
        target
            .capture_png(
                path,
                w,
                h,
                &mut self.session.camera,
                &mut self.session.registry,
                &self.session.settings,
                &self.session.named_palette,
                &self.session.palette,
                self.session.clear_color,
            )
            .map_err(|e| e.to_string())
    }

    fn set_viewport_gpu_image_from_wgpu_buffer(
        &mut self,
        buffer: &wgpu::Buffer,
        buffer_size: u64,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let target = self
            .render_context
            .as_deref_mut()
            .ok_or_else(|| "No render context available".to_string())?;
        target
            .set_viewport_gpu_image_from_buffer(buffer, buffer_size, width, height)
            .map_err(|e| e.to_string())?;
        self.session.viewport_image = None;
        *self.needs_redraw = true;
        Ok(())
    }

    fn save_viewport_gpu_image(&mut self, path: &Path) -> Result<Option<(u32, u32)>, String> {
        let Some(target) = self.render_context.as_deref_mut() else {
            return Ok(None);
        };
        target
            .save_viewport_gpu_image(path)
            .map_err(|e| e.to_string())
    }

    fn clear_viewport_gpu_image(&mut self) {
        if let Some(render_context) = self.render_context.as_deref_mut() {
            render_context.clear_viewport_gpu_image();
        }
    }

    #[cfg(feature = "render-bridge")]
    fn export_displayed_geometry(
        &mut self,
        options: &patinae_render::GeometryExportOptions,
    ) -> Result<patinae_render::DisplayedGeometry, String> {
        let target = self
            .render_context
            .as_deref_mut()
            .ok_or_else(|| "No render context available".to_string())?;
        target
            .export_displayed_geometry(
                &mut self.session.camera,
                &mut self.session.registry,
                &self.session.settings,
                &self.session.named_palette,
                &self.session.palette,
                self.session.clear_color,
                options,
            )
            .map_err(|e| e.to_string())
    }

    #[cfg(feature = "render-bridge")]
    fn for_each_trace_geometry_chunk(
        &mut self,
        options: &patinae_render::GeometryExportOptions,
        visitor: &mut dyn FnMut(patinae_render::TraceGeometryChunk) -> Result<(), String>,
    ) -> Result<(), String> {
        let target = self
            .render_context
            .as_deref_mut()
            .ok_or_else(|| "No render context available".to_string())?;
        target
            .for_each_trace_geometry_chunk(
                &mut self.session.camera,
                &mut self.session.registry,
                &self.session.settings,
                &self.session.named_palette,
                &self.session.palette,
                self.session.clear_color,
                options,
                visitor,
            )
            .map_err(|e| e.to_string())
    }

    #[cfg(feature = "render-bridge")]
    fn visit_render_artifacts(
        &mut self,
        visitor: &mut dyn FnMut(patinae_render::RenderArtifactSnapshot<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        let Some(render_context) = &mut self.render_context else {
            return Err("render artifact snapshot requires an active renderer".to_string());
        };

        render_context
            .visit_render_artifacts(
                &mut self.session.camera,
                &mut self.session.registry,
                &self.session.settings,
                &self.session.named_palette,
                &self.session.palette,
                self.session.clear_color,
                visitor,
            )
            .map_err(|e| e.to_string())
    }

    fn gpu_device(&self) -> Option<&wgpu::Device> {
        // `Arc::as_ref(&Arc<T>) -> &T` deref to the underlying device
        // without bumping the refcount.
        self.render_context
            .as_deref()
            .map(|c| c.gpu_device().as_ref())
    }

    fn gpu_queue(&self) -> Option<&wgpu::Queue> {
        self.render_context
            .as_deref()
            .map(|c| c.gpu_queue().as_ref())
    }

    fn prepare_render(&mut self) {
        // Per-frame rep prep happens on the host side (patinae's
        // `state.sync()` already rebuilds dirty reps every frame). Commands
        // that explicitly want a pre-upload (rare) can request a redraw via
        // `request_redraw` and the host's next frame picks it up.
    }

    fn capture_frame_png(
        &mut self,
        frame: usize,
        path: &Path,
        width: Option<u32>,
        height: Option<u32>,
    ) -> Result<(), String> {
        self.session.movie.goto_frame(frame);
        self.session.sync_movie_frame();
        self.capture_png(path, width, height)
    }

    // =========================================================================
    // Per-impl methods (borrow checker constraints)
    // =========================================================================

    fn scene_store(&mut self, key: &str, storemask: u32) {
        let mask = SceneStoreMask::from_bits_truncate(storemask);
        self.session
            .scenes
            .store(key, mask, &self.session.camera, &self.session.registry);
        *self.needs_redraw = true;
    }

    fn scene_recall(&mut self, key: &str, animate: bool, duration: f32) -> Result<(), String> {
        self.session
            .scenes
            .recall(
                key,
                &mut self.session.camera,
                &mut self.session.registry,
                animate,
                duration,
            )
            .map_err(|e| e.to_string())?;
        *self.needs_redraw = true;
        Ok(())
    }

    fn view_recall(&mut self, key: &str, animate: f32) -> Result<(), String> {
        self.session
            .views
            .recall(key, &mut self.session.camera, animate)
            .map_err(|e| e.to_string())?;
        *self.needs_redraw = true;
        Ok(())
    }

    fn viewport_size(&self) -> (u32, u32) {
        self.default_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::MoleculeObject;
    use crate::viewer_trait::ViewerLike;
    use crate::ViewerError;
    use lin_alg::f32::Vec3;
    use patinae_mol::{Atom, CoordSet, Element, ObjectMolecule};
    use patinae_settings::groups::RecentPickLimit;
    use std::sync::Arc;

    struct StateCheckingRenderer {
        expected_state: usize,
        seen_state: Option<usize>,
    }

    impl CaptureRenderer for StateCheckingRenderer {
        fn gpu_device(&self) -> &Arc<wgpu::Device> {
            panic!("gpu_device is not used by this test")
        }

        fn gpu_queue(&self) -> &Arc<wgpu::Queue> {
            panic!("gpu_queue is not used by this test")
        }

        fn capture_png(
            &mut self,
            _path: &Path,
            _width: u32,
            _height: u32,
            _camera: &mut Camera,
            registry: &mut ObjectRegistry,
            _settings: &Settings,
            _named: &NamedPalette,
            _themed: &patinae_color::ThemedPalette,
            _clear_color: [f32; 3],
        ) -> Result<(), ViewerError> {
            let state = registry.get_molecule("mol").unwrap().display_state();
            self.seen_state = Some(state);
            assert_eq!(state, self.expected_state);
            Ok(())
        }
    }

    fn multi_state_molecule() -> ObjectMolecule {
        let mut mol = ObjectMolecule::new("mol");
        mol.add_atom(Atom::new("C", Element::Carbon));
        for state in 0..3 {
            mol.add_coord_set(CoordSet::from_vec3(&[Vec3::new(state as f32, 0.0, 0.0)]));
        }
        mol
    }

    #[test]
    fn capture_frame_png_syncs_mset_state_before_capture() {
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(multi_state_molecule(), "mol"));
        session.movie.set_from_spec(vec![1, 2, 3]);

        let mut renderer = StateCheckingRenderer {
            expected_state: 2,
            seen_state: None,
        };
        let mut needs_redraw = false;

        {
            let mut adapter = SessionAdapter {
                session: &mut session,
                render_context: Some(&mut renderer),
                default_size: (64, 64),
                needs_redraw: &mut needs_redraw,
                async_fetch_fn: None,
            };
            adapter
                .capture_frame_png(2, Path::new("/tmp/movie-frame.png"), None, None)
                .unwrap();
        }

        assert_eq!(renderer.seen_state, Some(2));
    }

    #[test]
    fn replace_session_invalidates_generation_tokens_on_collision() {
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(multi_state_molecule(), "mol"));
        session
            .recent_atoms
            .insert("stale", RecentPickLimit::Unlimited);
        let old_registry_generation = session.registry.generation();
        let old_selection_generation = session.selections.generation();
        let old_recent_generation = session.recent_atoms.generation();
        let old_recent_incarnation = session.recent_atoms.incarnation();
        let mut replacement = Session::new();
        replacement
            .registry
            .add(MoleculeObject::with_name(multi_state_molecule(), "mol"));
        let path = crate::canonical_atom_path_for_hit(
            &crate::PickHit {
                object_name: "mol".to_string(),
                object_type: crate::ObjectType::Molecule,
                atom_index: Some(patinae_mol::AtomIndex(0)),
                position: Vec3::new(0.0, 0.0, 0.0),
                distance: 0.0,
            },
            replacement.registry.get_molecule("mol").unwrap().molecule(),
        )
        .unwrap();
        replacement
            .recent_atoms
            .insert(path.clone(), RecentPickLimit::Unlimited);
        let mut needs_redraw = false;

        {
            let mut adapter = SessionAdapter {
                session: &mut session,
                render_context: None,
                default_size: (64, 64),
                needs_redraw: &mut needs_redraw,
                async_fetch_fn: None,
            };
            adapter.replace_session(replacement);
        }

        assert_ne!(session.registry.generation(), old_registry_generation);
        assert_ne!(session.selections.generation(), old_selection_generation);
        assert_ne!(session.recent_atoms.generation(), old_recent_generation);
        assert_ne!(
            session.recent_atoms.incarnation(),
            old_recent_incarnation,
            "replacement must invalidate row identity even when restored IDs collide"
        );
        assert_eq!(session.recent_atoms.paths().collect::<Vec<_>>(), [path]);
        assert!(needs_redraw);
    }
}
