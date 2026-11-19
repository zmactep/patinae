//! Browser PNG encoding and projection of the scene's text annotations.

use std::cell::RefCell;
use std::rc::Rc;

use js_sys::Promise;
use patinae_scene::bridge::{project_frame_labels, ProjectedAnnotationLabel};
use wasm_bindgen::{prelude::*, Clamped};
use wasm_bindgen_futures::{future_to_promise, JsFuture};
use web_sys::{ImageData, OffscreenCanvas, OffscreenCanvasRenderingContext2d};

use super::{WebState, WebViewer};
use crate::render_loop::capture;

#[wasm_bindgen]
impl WebViewer {
    /// Renders the current scene and resolves to a PNG Blob.
    ///
    /// # Errors
    ///
    /// Rejects invalid sizes, overlapping exports, GPU errors, or a closed viewer.
    pub fn export_png(
        &self,
        width: Option<f64>,
        height: Option<f64>,
        transparent: Option<bool>,
    ) -> Result<Promise, JsValue> {
        start(
            &self.state,
            pixel_dimension(width, "width")?,
            pixel_dimension(height, "height")?,
            transparent,
            None,
        )
    }
}

fn pixel_dimension(value: Option<f64>, name: &str) -> Result<Option<u32>, JsValue> {
    value
        .map(|v| {
            if v.fract() != 0.0 || !(1.0..=f64::from(u32::MAX)).contains(&v) {
                return Err(js_sys::RangeError::new(&format!(
                    "PNG {name} must be a positive 32-bit integer, received {v}"
                ))
                .into());
            }
            Ok(v as u32)
        })
        .transpose()
}

/// Submits the scene while the command's camera and coordinate state are current.
pub(super) fn start(
    state: &Rc<RefCell<WebState>>,
    width: Option<u32>,
    height: Option<u32>,
    transparent: Option<bool>,
    cancel: Option<web_sys::AbortSignal>,
) -> Result<Promise, JsValue> {
    let mut host = state.borrow_mut();
    if host.png_in_progress {
        return Err(js_sys::Error::new("A PNG export is already in progress").into());
    }
    host.prepare_frame_memory().map_err(js_sys::Error::new)?;
    let WebState {
        gpu,
        session,
        render_scene,
        needs_redraw,
        ..
    } = &mut *host;
    let gpu = gpu
        .as_mut()
        .ok_or_else(|| js_sys::Error::new("PNG export requires an initialized WebGPU device"))?;
    *needs_redraw = true;
    let pending = capture::begin(gpu, session, render_scene, width, height, transparent)
        .map_err(|error| js_sys::Error::new(&error))?;

    let (w, h) = (pending.width, pending.height);
    let scale = host.display_scale * (h as f32 / host.height.max(1) as f32);
    let labels = project_frame_labels(
        &pending.view_proj,
        (0.0, 0.0, w as f32, h as f32),
        host.render_scene.annotation_bundles(),
        scale,
    );

    host.png_in_progress = true;
    drop(host);
    let state = Rc::downgrade(state);
    Ok(future_to_promise(async move {
        let cancelled = || {
            cancel.as_ref().is_some_and(web_sys::AbortSignal::aborted)
                || !state.upgrade().is_some_and(|host| !host.borrow().closed)
        };
        let result = async {
            let rgba = pending
                .rgba(cancelled)
                .await
                .map_err(|error| js_sys::Error::new(&error))?;
            if cancelled() {
                return Err(js_sys::Error::new("PNG export cancelled").into());
            }
            let encoded = encode(w, h, &rgba, labels, scale)?;
            drop(rgba);
            let blob = JsFuture::from(encoded).await?;
            if cancelled() {
                return Err(js_sys::Error::new("PNG export cancelled").into());
            }
            Ok(blob)
        }
        .await;
        if let Some(host) = state.upgrade() {
            let mut host = host.borrow_mut();
            host.png_in_progress = false;
            if result.is_err() {
                if let Some(gpu) = &mut host.gpu {
                    gpu.state.discard_capture_targets();
                }
            }
            if !host.closed {
                return result;
            }
        }
        Err(js_sys::Error::new("Viewer closed during PNG export").into())
    }))
}

fn encode(
    width: u32,
    height: u32,
    rgba: &[u8],
    labels: Vec<ProjectedAnnotationLabel>,
    scale: f32,
) -> Result<Promise, JsValue> {
    let canvas = OffscreenCanvas::new(width, height)?;
    let context: OffscreenCanvasRenderingContext2d = canvas
        .get_context("2d")?
        .ok_or_else(|| js_sys::Error::new("Could not allocate a 2D canvas for PNG encoding"))?
        .dyn_into()?;
    let pixels = ImageData::new_with_u8_clamped_array_and_sh(Clamped(rgba), width, height)?;
    context.put_image_data(&pixels, 0.0, 0.0)?;

    // Match the live label overlay's font stack, anchor and contrast shadow.
    context.set_shadow_color("rgba(0,0,0,0.8)");
    context.set_shadow_blur(3.0 * f64::from(scale));
    for label in labels {
        let size = label.size * scale;
        context.set_font(&format!(
            "{size}px -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif"
        ));
        let [r, g, b, a] = label.color;
        let channel = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        context.set_fill_style_str(&format!(
            "rgba({},{},{},{})",
            channel(r),
            channel(g),
            channel(b),
            a.clamp(0.0, 1.0)
        ));
        let metrics = context.measure_text(&label.text)?;
        let (anchor_x, anchor_y) = label.alignment.anchor_factors();
        let ascent = metrics.font_bounding_box_ascent();
        let line_height = ascent + metrics.font_bounding_box_descent();
        let x = f64::from(label.x) - f64::from(anchor_x) * metrics.width();
        let y = f64::from(label.y) - f64::from(anchor_y) * line_height + ascent;
        context.fill_text(&label.text, x, y)?;
    }
    // The browser's default OffscreenCanvas encoder is image/png.
    canvas.convert_to_blob()
}
