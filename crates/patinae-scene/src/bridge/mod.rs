//! Bridges from `Session` state to `patinae_render::RenderInput`.
//!
//! Used by both `patinae` (Slint viewport) and `web/` (WASM viewer) so the
//! same scene state populates both renderers identically. Gated by the
//! `render-bridge` feature — headless consumers (pure scene parsing /
//! command-line tools) can disable to drop the `patinae-render` dependency.
//!
//! ## Lifetime shape
//!
//! `RenderInput` borrows everything. Build the resolved color buffers
//! first (they own their data), then walk the registry and emit
//! `RenderObjectInput`s referencing both the registry and the color
//! buffers:
//!
//! ```ignore
//! let mut cache = CachedRenderScene::default();
//! let frame = cache.prepare(&mut session);
//! state.sync(&frame.render_input());
//! ```
//!
//! Color resolution happens here on the host: per-atom base RGBA plus
//! representation-specific packed RGB overrides are pre-computed through
//! `patinae_color::ColorResolver` and handed to the renderer in flat vectors
//! indexed by `AtomIndex`. The renderer applies per-rep alpha (e.g.
//! `sphere_transparency`) on top.

mod annotations;
mod cache;
mod colors;
mod frame;
mod markers;
mod objects;
mod picking;
mod strokes;

pub use annotations::{
    project_annotation_labels, ProjectedAnnotationLabel, ProjectedSceneLabels,
    ResolvedSceneAnnotations,
};
pub use cache::{CachedRenderFrame, CachedRenderScene};
pub use colors::{resolve_setting_color, ResolvedSceneColors};
pub use frame::{frame_uniforms_from_camera, frame_uniforms_from_session};
pub use markers::ResolvedSceneMarkers;
pub use objects::{visit_render_objects, visit_render_scene};
pub use picking::resolve_pick;
pub use strokes::ResolvedSceneStrokes;
