use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use patinae_framework::component::SharedContext;
use patinae_framework::kernel::AppKernel;
use patinae_framework::message::AppMessage;
use patinae_framework::plugin_ui::{PanelEvent, PanelEventKind, PanelValue};
use patinae_framework::topics::{self, SaveFileRequest, SAVE_FILE_REQUEST_TOPIC};
use patinae_plugin_host::PluginHost;
use patinae_render::{
    bytes_to_mib, required_limits_for_memory_policy, FrameStatsHistory, RenderMemoryPolicy,
    RenderMemoryProfile, RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE,
};
use patinae_scene::{
    bridge::ProjectedAnnotationLabel, expand_pick_to_selection, pick_expression_for_hit,
    CaptureRenderer, KeyBinding, ViewportImage,
};
use patinae_select::build_sele_command;
use patinae_settings::{
    paths::{PatinaercPath, PatinaercSource},
    ThemeMode,
};

use crate::bridges::input::{InputBridge, PointerSnapshot};
use crate::bridges::movie::MovieBridge;
use crate::bridges::objects::ObjectsBridge;
use crate::bridges::platform::PlatformBridge;
use crate::bridges::plugins::PluginBridge;
use crate::bridges::repl::ReplBridge;
use crate::bridges::sequence::SequenceBridge;
use crate::bridges::viewport::ViewportBridge;
use crate::components::viewport::{render_memory_policy_for_device, ViewportRenderer};
#[cfg(not(target_os = "windows"))]
use crate::display_recovery::log_selected_gpu;
use crate::display_recovery::{
    log_window_event, mark_display_transition_and_redraw, request_display_recovery_redraw,
    DisplayRecovery,
};
use crate::native_file_actions::{enqueue_file_action, quote_command_arg, NativeFileSource};
use crate::recent_files::RecentFilesBridge;
use crate::recent_thumbnails::{RECENT_THUMBNAIL_HEIGHT, RECENT_THUMBNAIL_WIDTH};
use crate::startup_alert;
use crate::{
    AppWindow, LayoutState, MenuState, NotificationState, Theme, ViewportLabel, ViewportState,
};

const SIDEBAR_WIDTH_LP: f32 = 48.0;
// Keep these in sync with `patinae/ui/tokens.slint`; raw winit input is
// filtered before Slint hit-testing can tell Rust which overlay consumed it.
const REPL_PILL_WIDTH_LP: f32 = 380.0;
const REPL_PILL_HEIGHT_LP: f32 = 32.0;
const REPL_PILL_BOTTOM_LP: f32 = 24.0;
const TRANSIENT_NOTIFICATION_SECS: u64 = 2;

fn format_mib(bytes: u64) -> String {
    format!("{:.2} MiB", bytes_to_mib(bytes))
}

fn format_hud_mib(bytes: u64) -> String {
    let mib = bytes_to_mib(bytes);
    if mib >= 100.0 {
        format!("{mib:.0} MiB")
    } else {
        format!("{mib:.1} MiB")
    }
}

fn memory_profile_hud_label(profile: RenderMemoryProfile) -> &'static str {
    match profile {
        RenderMemoryProfile::Performance => "PERF",
        RenderMemoryProfile::Balanced => "BAL",
        RenderMemoryProfile::Lite => "LITE",
        RenderMemoryProfile::Manual { .. } => "MAN",
    }
}

fn memory_total_hud_text(total_bytes: u64, budget_bytes: Option<u64>) -> String {
    let total = format_hud_mib(total_bytes);
    match budget_bytes {
        Some(budget_bytes) => format!("{total} / {}", format_hud_mib(budget_bytes)),
        None => total,
    }
}

/// Fraction of the memory budget currently allocated, clamped to `0..=1`.
/// Returns `-1.0` when the profile has no budget (unbounded) so the HUD can
/// hide the usage bar.
fn memory_budget_fill(total_bytes: u64, budget_bytes: Option<u64>) -> f32 {
    match budget_bytes {
        Some(budget) if budget > 0 => (total_bytes as f32 / budget as f32).clamp(0.0, 1.0),
        _ => -1.0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupAction {
    Warning(String),
    RunPatinaerc(PathBuf),
    RouteArgvFile(PathBuf),
}

// ---------------------------------------------------------------------------
// App — thin orchestrator wiring kernel + bridges + renderer
// ---------------------------------------------------------------------------

pub struct App {
    pub(crate) kernel: AppKernel,
    renderer: Option<ViewportRenderer>,
    input: InputBridge,
    pub(crate) objects: ObjectsBridge,
    pub(crate) movie: MovieBridge,
    pub(crate) repl: ReplBridge,
    pub(crate) sequence: SequenceBridge,
    pub(crate) plugin_bridge: PluginBridge,
    pub(crate) plugins: PluginHost,
    recent_files: RecentFilesBridge,
    pending_recent_thumbnail: Option<PathBuf>,
    command_names_cache: Vec<String>,
    setting_names_cache: Vec<&'static str>,
    last_plugin_sync_key: Option<u64>,
    viewport: ViewportBridge,
    platform: PlatformBridge,
    prev_theme: ThemeMode,
    display_recovery: Rc<Cell<DisplayRecovery>>,
    /// Shared accumulator for trackpad pinch-zoom gestures (macOS).
    /// Written by `on_winit_window_event`, consumed in `render_frame`.
    pub(crate) pinch_accumulator: Rc<Cell<f64>>,
    /// Shared modifier state from winit's `ModifiersChanged` event.
    /// `(shift, ctrl, alt, super)`. Written by `on_winit_window_event`,
    /// consumed in `render_frame`.
    pub(crate) winit_modifiers: Rc<Cell<(bool, bool, bool, bool)>>,
    /// Set when the window loses focus — triggers full input reset in `render_frame`.
    pub(crate) window_focus_lost: Rc<Cell<bool>>,
    /// Raw cursor position from winit's `CursorMoved` event (physical pixels,
    /// window-relative). Used for hover picking since Slint's `moved` callback
    /// only fires during drag (button pressed).
    pub(crate) cursor_pos: Rc<Cell<(f32, f32)>>,
    /// Whether `cursor_pos` has been initialized by a real winit
    /// `CursorMoved`. The first mouse press after startup/focus can arrive
    /// before any movement event, in which case Slint's viewport-local
    /// pointer state is the only trustworthy coordinate source.
    pub(crate) cursor_known: Rc<Cell<bool>>,
    /// Raw mouse button state from winit. Slint's pointer globals can update
    /// too late for the first drag frame, so camera input consumes this path.
    pub(crate) raw_mouse_buttons: Rc<Cell<(bool, bool, bool)>>,
    /// Window-relative press position for the currently-active raw drag.
    pub(crate) raw_press_pos: Rc<Cell<(f32, f32)>>,
    pub(crate) raw_press_known: Rc<Cell<bool>>,
    /// `true` when `PATINAE_PERF=1` was set at construct time. Drives the
    /// HUD overlay populated by `update_perf_hud`. Plain frame-time
    /// history (no GPU stats) is collected unconditionally so a future
    /// in-app toggle could flip this on without losing recent data.
    perf_enabled: bool,
    /// Last `render_frame` `Instant`. The inter-call delta is the
    /// observed end-to-end frame time and feeds `perf_history` —
    /// captures Slint idle + GPU present, not just our CPU portion.
    last_frame_instant: Cell<Option<Instant>>,
    /// Last animation tick time used for deterministic movie/rock updates.
    last_animation_instant: Cell<Option<Instant>>,
    /// Ring buffer of recent frame times. Always allocated so the HUD
    /// can show immediate data when toggled on.
    perf_history: RefCell<FrameStatsHistory>,
    /// Frame counter for throttling Slint property writes (refresh
    /// percentile string every N frames instead of every frame).
    perf_update_counter: Cell<u32>,
    /// Ordered startup work. Drained after the first frame so startup
    /// scripts and large loads do not block window creation.
    startup_actions: VecDeque<StartupAction>,
    /// Save-file picker requests emitted while rendering plugin panel events.
    ///
    /// Drained on a later event-loop tick so native modal dialogs cannot
    /// re-enter Slint rendering while the current surface image is acquired.
    pending_save_file_requests: VecDeque<SaveFileRequest>,
    save_file_dialog_scheduled: bool,
    transient_notification: Option<(String, Instant)>,
    last_empty_mode: bool,
}

impl App {
    pub fn new() -> Self {
        let perf_enabled = std::env::var("PATINAE_PERF")
            .ok()
            .map(|v| v != "0" && !v.is_empty())
            .unwrap_or(false);
        let mut kernel = AppKernel::new();
        kernel.set_async_command_handler(Some(Box::new(crate::fetch::task_from_request)));
        let mut plugins = PluginHost::new();
        plugins.load_standard_dirs(&mut kernel.executor);
        let command_names_cache = kernel
            .executor
            .registry()
            .names()
            .map(|s| s.to_string())
            .collect();
        let setting_names_cache = patinae_settings::setting_names();
        Self {
            kernel,
            renderer: None,
            input: InputBridge::new(),
            objects: ObjectsBridge::new(),
            movie: MovieBridge::new(),
            repl: ReplBridge::new(),
            sequence: SequenceBridge::new(),
            plugin_bridge: PluginBridge::new(),
            plugins,
            recent_files: RecentFilesBridge::new(
                patinae_settings::paths::recent_files_path(),
                patinae_settings::paths::thumbnail_cache_dir().join("recent"),
            ),
            pending_recent_thumbnail: None,
            command_names_cache,
            setting_names_cache,
            last_plugin_sync_key: None,
            viewport: ViewportBridge::new(800, 500),
            platform: PlatformBridge::new(),
            prev_theme: ThemeMode::Dark,
            display_recovery: Rc::new(Cell::new(DisplayRecovery::default())),
            pinch_accumulator: Rc::new(Cell::new(0.0)),
            winit_modifiers: Rc::new(Cell::new((false, false, false, false))),
            window_focus_lost: Rc::new(Cell::new(false)),
            cursor_pos: Rc::new(Cell::new((0.0, 0.0))),
            cursor_known: Rc::new(Cell::new(false)),
            raw_mouse_buttons: Rc::new(Cell::new((false, false, false))),
            raw_press_pos: Rc::new(Cell::new((0.0, 0.0))),
            raw_press_known: Rc::new(Cell::new(false)),
            perf_enabled,
            last_frame_instant: Cell::new(None),
            last_animation_instant: Cell::new(None),
            perf_history: RefCell::new(FrameStatsHistory::with_default_capacity()),
            perf_update_counter: Cell::new(0),
            startup_actions: VecDeque::new(),
            pending_save_file_requests: VecDeque::new(),
            save_file_dialog_scheduled: false,
            transient_notification: None,
            last_empty_mode: true,
        }
    }

    // --- Renderer lifecycle ---

    pub fn setup_renderer(&mut self, api: &slint::GraphicsAPI, app: &AppWindow) -> Option<()> {
        let (vw, vh) = Self::viewport_size(app);
        let (renderer, info) = ViewportRenderer::setup(api)?;
        log::info!("wgpu 28 device acquired — initializing viewport");
        self.viewport.resize(vw, vh);
        self.kernel.resize(vw, vh);
        self.renderer = Some(renderer);
        self.mark_display_transition();

        if let Some(info) = info {
            log::info!("GPU: {} ({})", info.device_name, info.backend);
            let vp = app.global::<ViewportState>();
            vp.set_device_name(info.device_name.into());
            vp.set_engine_name(info.backend.into());
        }

        Some(())
    }

    pub fn teardown_renderer(&mut self) {
        log::info!("Rendering teardown — dropping viewport");
        self.renderer = None;
    }

    fn mark_display_transition(&self) {
        let mut recovery = self.display_recovery.get();
        recovery.mark_transition();
        self.display_recovery.set(recovery);
    }

    fn take_live_texture_invalidation(&self) -> bool {
        let mut recovery = self.display_recovery.get();
        let invalid = recovery.take_live_texture_invalidation();
        self.display_recovery.set(recovery);
        invalid
    }

    pub(crate) fn submit_repl_command(&mut self, cmd: String, app: &AppWindow) {
        self.kernel.command_line.input = cmd;
        let viewport_size = Self::viewport_size(app);
        let rc: Option<&mut dyn CaptureRenderer> = self
            .renderer
            .as_mut()
            .map(|r| r as &mut dyn CaptureRenderer);
        self.kernel.submit_command(rc, viewport_size);
    }

    /// Route a native menu Open selection through the shared file router.
    #[cfg(target_os = "macos")]
    pub fn route_menu_open_file(&mut self, path: PathBuf) {
        self.route_native_file(path.as_path(), NativeFileSource::MenuOpen);
    }

    /// Route a native menu Run Script selection through the shared file router.
    #[cfg(target_os = "macos")]
    pub fn route_menu_run_script(&mut self, path: PathBuf) {
        self.route_native_file(path.as_path(), NativeFileSource::MenuRunScript);
    }

    fn route_dropped_file(&mut self, path: &std::path::Path) {
        self.route_native_file(path, NativeFileSource::DragDrop);
    }

    fn route_native_file(&mut self, path: &std::path::Path, source: NativeFileSource) {
        enqueue_file_action(&mut self.kernel, path, source);
    }

    pub(crate) fn open_recent_file(&mut self, app: &AppWindow, path: &str) {
        let path = PathBuf::from(path);
        match path.try_exists() {
            Ok(true) => {
                self.kernel
                    .bus
                    .execute_command(recent_open_command_for_existing_path(&path));
            }
            Ok(false) => {
                if let Err(err) = self.recent_files.remove(&path, app) {
                    self.kernel.bus.print_warning(format!(
                        "Could not update recent files after missing file: {err}"
                    ));
                }
                self.kernel.bus.print_warning(format!(
                    "Recent file is no longer available: {}",
                    path.display()
                ));
            }
            Err(err) => {
                self.kernel.bus.print_warning(format!(
                    "Could not check recent file {}: {err}",
                    path.display()
                ));
            }
        }
        self.kernel.bus.request_redraw();
        app.window().request_redraw();
    }

    // --- Per-frame rendering ---

    pub fn render_frame(&mut self, app: &AppWindow) {
        if self.renderer.is_none() {
            return;
        }

        // PATINAE_TIMING=1 → per-section timing dump on every frame.
        // Helps identify where the host loop spends time.
        let timing = std::env::var("PATINAE_TIMING").is_ok();
        let t_total = if timing {
            Some(std::time::Instant::now())
        } else {
            None
        };
        // End-to-end frame time = gap between consecutive `render_frame`
        // invocations. Captures Slint idle + present wait + GPU stall —
        // the wall-clock signal a user actually perceives. Fed
        // unconditionally into `perf_history` so the HUD has data the
        // moment `PATINAE_PERF=1` is toggled.
        let now = Instant::now();
        if let Some(prev) = self.last_frame_instant.get() {
            let gap_ms = now.duration_since(prev).as_secs_f32() * 1000.0;
            if gap_ms > 0.0 {
                self.perf_history.borrow_mut().push(gap_ms);
            }
            if timing && gap_ms > 1.0 {
                eprintln!("[patinae] inter-frame gap: {gap_ms:.2} ms");
            }
        }
        self.last_frame_instant.set(Some(now));
        let mut t_section = std::time::Instant::now();
        let mark = |label: &str, t: &mut std::time::Instant| {
            if !timing {
                return;
            }
            let now = std::time::Instant::now();
            let dt = now.duration_since(*t).as_secs_f32() * 1000.0;
            if dt > 0.05 {
                eprintln!("  [patinae] {label}: {dt:.2} ms");
            }
            *t = now;
        };

        let vp = app.global::<ViewportState>();
        let sf = app.window().scale_factor();
        let (vw, vh) = Self::viewport_size(app);
        let fetch_dialog_visible = app.global::<MenuState>().get_fetch_visible();
        mark("setup", &mut t_section);

        // Resize + Input + Theme + Platform
        self.viewport.resize(vw, vh);
        self.kernel.resize(vw, vh);
        if self.take_live_texture_invalidation() {
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.invalidate_live_textures();
            }
        }
        // Reset all input on window focus loss (OS may swallow mouse-up events)
        if self.window_focus_lost.get() {
            self.kernel.viewport.input.reset();
            self.input = InputBridge::new();
            self.window_focus_lost.set(false);
        }

        let pointer = self.pointer_snapshot(
            &vp,
            vw,
            vh,
            sf,
            app.global::<LayoutState>().get_repl_visible(),
            fetch_dialog_visible,
        );
        self.input.sync(
            &mut self.kernel.viewport.input,
            &vp,
            sf,
            self.winit_modifiers.get(),
            Some(pointer),
        );

        // Consume accumulated pinch-zoom gesture (macOS trackpad)
        let pinch = self.pinch_accumulator.get();
        if pinch.abs() > 0.0 {
            self.kernel.viewport.input.handle_pinch_zoom(pinch);
            self.pinch_accumulator.set(0.0);
        }

        let theme_changed = crate::bridges::theme::sync_theme(
            &mut self.kernel,
            &app.global::<Theme>(),
            app,
            &mut self.prev_theme,
        );
        if theme_changed {
            self.kernel.session.registry.mark_all_dirty();
        }
        self.platform.sync(app);

        if fetch_dialog_visible {
            self.kernel.viewport.input.reset();
            let _ = self.input.take_pending_click();
            if self.kernel.viewport.hover_hit.is_some() {
                self.clear_hover_indicators();
            }
        } else {
            // Camera
            if self.kernel.process_input(vh as f32) {
                self.dismiss_viewport_image();
            }

            // Picking: click → selection command, hover → indicators.
            // Skip hover on click frame to avoid yellow flash before selection takes effect.
            // PATINAE_NO_HOVER=1 disables hover picking entirely (diagnostic).
            let hover_disabled = std::env::var("PATINAE_NO_HOVER")
                .ok()
                .map(|v| v == "1")
                .unwrap_or(false);
            if let Some(click_pos) = self.input.take_pending_click() {
                self.process_click(click_pos, (vw, vh));
                // Clear hover state so it re-evaluates cleanly next frame
                self.clear_hover_indicators();
            } else if !hover_disabled {
                self.process_hover(vw, vh, sf);
            }
        }
        mark("input+pick", &mut t_section);

        self.poll_plugins((vw, vh));
        mark("plugins.poll", &mut t_section);

        self.kernel.process_async_tasks();
        mark("async.tasks", &mut t_section);
        self.sync_notifications(app);

        // Drain pending commands (color, show, etc. — sets dirty flags on molecules).
        // Cast the renderer to `&mut dyn CaptureRenderer` for the trait-based
        // command path; commands that need GPU access (just `png` /
        // `movie render`) call back through the trait.
        let rc: Option<&mut dyn CaptureRenderer> = self
            .renderer
            .as_mut()
            .map(|r| r as &mut dyn CaptureRenderer);
        let unhandled = self.kernel.process_messages(rc, (vw, vh));
        dispatch_lifecycle_messages(&unhandled, slint::quit_event_loop);
        mark("commands", &mut t_section);

        let mut forwarded_messages = Vec::with_capacity(unhandled.len());
        for msg in &unhandled {
            if crate::native_menu::dispatch_metadata_message(msg, app) {
                continue;
            }
            if self.dispatch_recent_file_message(msg, app) {
                continue;
            }
            if self.queue_save_file_request(msg) {
                continue;
            }
            self.plugins.broadcast(msg, &mut self.kernel.bus);
            forwarded_messages.push(msg.clone());
        }
        let layout_messages: Vec<_> = forwarded_messages
            .iter()
            .filter_map(|msg| {
                if self.dispatch_plugin_layout_message(msg) {
                    None
                } else {
                    Some(msg.clone())
                }
            })
            .collect();

        let animation_dt = self
            .last_animation_instant
            .get()
            .map(|prev| now.duration_since(prev).as_secs_f32().min(0.1))
            .unwrap_or(0.0);
        self.last_animation_instant.set(Some(now));
        self.kernel.update_animations(animation_dt);
        mark("animations", &mut t_section);

        // Dispatch layout messages (ShowPanel, HidePanel, TogglePanel)
        if !layout_messages.is_empty() {
            crate::bridges::layout::dispatch_messages(&layout_messages, app);
        }
        self.sync_empty_mode(app);

        self.sync_plugins(app);
        mark("plugins.ui", &mut t_section);

        // Sync objects panel (sees dirty flags before render clears them)
        self.objects.sync(&mut self.kernel, app);
        mark("objects panel", &mut t_section);

        // Sync sequence panel
        self.sequence.sync(&self.kernel, app);
        mark("sequence panel", &mut t_section);

        // Sync movie panel
        let movie_object = self.objects.single_movie_keyframe_object();
        self.movie.sync(&self.kernel, app, movie_object);
        mark("movie panel", &mut t_section);

        // Sync REPL output model
        self.repl.sync(&self.kernel, app);
        mark("repl panel", &mut t_section);

        // Render (rebuilds GPU reps from dirty flags, then clears them)
        let renderer = self.renderer.as_mut().unwrap();
        let render_outcome = renderer.render(&mut self.kernel.session, vw, vh);
        mark("renderer.render", &mut t_section);
        if let Some(warning) = render_outcome.warning {
            self.kernel.bus.print_warning(warning);
        }
        for warning in renderer.take_memory_warnings() {
            self.kernel.bus.print_warning(warning);
        }
        let annotation_labels = if self.kernel.session.viewport_image.is_some() {
            renderer.clear_annotation_labels();
            Vec::new()
        } else {
            let viewport = (0.0, 0.0, vw as f32, vh as f32);
            renderer
                .annotation_labels(&self.kernel.session.camera, viewport)
                .iter()
                .map(|label| viewport_label_from_projected(label, sf))
                .collect()
        };
        self.viewport.set_labels(annotation_labels);
        if render_outcome.request_redraw {
            self.kernel.bus.request_redraw();
            app.window().request_redraw();
        }
        if render_outcome.present_frame {
            let image = viewport_image_to_slint(self.kernel.session.viewport_image.as_ref())
                .or(render_outcome.image);
            self.viewport.push_frame(image, &vp);
            mark("push_frame", &mut t_section);
        }

        // Drain GPU stats once per frame; both PATINAE_TIMING (log) and
        // PATINAE_PERF (HUD) consume the same snapshot.
        let gpu_stats = self
            .renderer
            .as_ref()
            .and_then(|r| r.take_frame_stats_for_log());

        if let Some(t0) = t_total {
            let total_ms = t0.elapsed().as_secs_f32() * 1000.0;
            eprintln!("[patinae] render_frame TOTAL: {total_ms:.2} ms");
            if let Some(stats) = gpu_stats.as_ref() {
                eprintln!(
                    "  [gpu] shadow={:.2}  opaque={:.2}  translucent={:.2}  composite={:.2}  \
	                     marking={:.2}  picking={:.2}  reproject={:.2}  silhouette={:.2}  \
	                     cull={:.2}  atlas_ao={:.2}  total={:.2} ms",
                    stats.gpu_shadow_ms,
                    stats.gpu_opaque_ms,
                    stats.gpu_translucent_ms,
                    stats.gpu_composite_ms,
                    stats.gpu_marking_ms,
                    stats.gpu_picking_ms,
                    stats.gpu_picking_reproject_ms,
                    stats.gpu_silhouette_ms,
                    stats.gpu_cull_ms,
                    stats.gpu_atlas_ao_ms,
                    stats.gpu_total_ms,
                );
            }
            if let Some(memory) = self.renderer.as_ref().map(|r| r.memory_snapshot()) {
                eprintln!("  {}", memory.timing_line());
            }
            if let Some(timings) = self.renderer.as_ref().map(|r| r.last_sync_timings()) {
                let scene = timings.scene_store_fragmentation;
                eprintln!(
                    "  [gpu-mem] scene_store live={} allocated={} orphaned={} capacity={} slack={}",
                    format_mib(scene.live_bytes),
                    format_mib(scene.allocated_bytes),
                    format_mib(scene.orphaned_bytes),
                    format_mib(scene.capacity_bytes),
                    format_mib(scene.capacity_slack_bytes),
                );
                if timings.scene_store_compaction.ran {
                    let compaction = timings.scene_store_compaction;
                    let largest = compaction
                        .largest_orphaned_buffer
                        .map(|kind| kind.label())
                        .unwrap_or("none");
                    eprintln!(
                        "  [gpu-mem] scene_store compacted reclaimed={} capacity_before={} \
                         capacity_after={} orphaned_before={} moved_objects={} largest_orphaned={}",
                        format_mib(compaction.reclaimed_bytes),
                        format_mib(compaction.capacity_before_bytes),
                        format_mib(compaction.capacity_after_bytes),
                        format_mib(compaction.orphaned_before_bytes),
                        compaction.moved_objects,
                        largest,
                    );
                }
            }
        }

        if self.perf_enabled {
            self.update_perf_hud(&vp, gpu_stats.as_ref());
        }

        self.capture_pending_recent_thumbnail(app);
        self.run_startup_actions(app, (vw, vh));
    }

    fn poll_plugins(&mut self, viewport_size: (u32, u32)) {
        if self.plugins.plugin_count() == 0 {
            return;
        }

        {
            let gpu_device = self.renderer.as_ref().map(|r| r.gpu_device().as_ref());
            let gpu_queue = self.renderer.as_ref().map(|r| r.gpu_queue().as_ref());
            let kernel = &mut self.kernel;
            let scene_generation = kernel
                .session
                .registry
                .generation()
                .wrapping_add(kernel.session.selections.generation().rotate_left(1))
                .wrapping_add(kernel.session.movie.generation().rotate_left(2));
            let shared = SharedContext {
                registry: &kernel.session.registry,
                camera: &kernel.session.camera,
                selections: &kernel.session.selections,
                named_palette: &kernel.session.named_palette,
                movie: &kernel.session.movie,
                settings: &kernel.session.settings,
                clear_color: kernel.session.clear_color,
                gpu_device,
                gpu_queue,
                scene_generation,
                viewport_image: kernel.session.viewport_image.as_ref(),
                command_names: &self.command_names_cache,
                command_registry: kernel.executor.registry(),
                setting_names: &self.setting_names_cache,
                dynamic_settings: Some(kernel.executor.dynamic_settings()),
            };
            self.plugins.poll_all(&shared, &mut kernel.bus);
        }

        let mut results = Vec::new();
        for request in self.plugins.take_pending_executions() {
            let rc: Option<&mut dyn CaptureRenderer> = self
                .renderer
                .as_mut()
                .map(|r| r as &mut dyn CaptureRenderer);
            let result = self
                .kernel
                .execute_command(&request.command, request.silent, rc, viewport_size)
                .map(|_| ())
                .map_err(|e| e.to_string());
            results.push(patinae_plugin_host::CommandResult {
                id: request.id,
                result,
            });
        }
        self.plugins.store_command_results(results);

        for mutation in self.plugins.take_pending_mutations() {
            let rc: Option<&mut dyn CaptureRenderer> = self
                .renderer
                .as_mut()
                .map(|r| r as &mut dyn CaptureRenderer);
            self.kernel.mutate_viewer(rc, viewport_size, mutation);
        }

        if self
            .plugins
            .apply_dynamic_command_changes(&mut self.kernel.executor)
        {
            self.refresh_command_names_cache();
            self.plugins.invalidate_panel_ui();
        }
        self.plugins.apply_hotkey_changes();
    }

    fn dispatch_plugin_layout_message(
        &mut self,
        msg: &patinae_framework::message::AppMessage,
    ) -> bool {
        use patinae_framework::message::AppMessage;

        match msg {
            AppMessage::ShowPanel(id) if self.plugins.has_panel(id) => {
                self.plugins.show_panel(id);
                true
            }
            AppMessage::HidePanel(id) if self.plugins.has_panel(id) => {
                self.plugins.hide_panel(id);
                true
            }
            AppMessage::TogglePanel(id) if self.plugins.has_panel(id) => {
                self.plugins.toggle_panel(id);
                true
            }
            AppMessage::ActivateTab(id) if self.plugins.has_panel(id) => {
                self.plugins.activate_panel(id);
                true
            }
            _ => false,
        }
    }

    fn queue_save_file_request(&mut self, msg: &AppMessage) -> bool {
        let Some(request) = topics::subscribe::<SaveFileRequest>(msg, SAVE_FILE_REQUEST_TOPIC)
        else {
            return false;
        };

        self.pending_save_file_requests.push_back(request);
        true
    }

    fn dispatch_recent_file_message(&mut self, msg: &AppMessage, app: &AppWindow) -> bool {
        let AppMessage::RecordRecentFile { path, command } = msg else {
            return false;
        };

        if command != "load" {
            return true;
        }

        if let Err(err) = self.recent_files.record(PathBuf::from(path), app) {
            self.kernel
                .bus
                .print_warning(format!("Could not save recent files: {err}"));
        } else {
            self.pending_recent_thumbnail = Some(PathBuf::from(path));
        }
        true
    }

    fn capture_pending_recent_thumbnail(&mut self, app: &AppWindow) {
        let Some(path) = self.pending_recent_thumbnail.take() else {
            return;
        };
        if self.kernel.session.registry.is_empty() {
            return;
        }

        let paths = match self.recent_files.thumbnail_write_paths(&path) {
            Ok(paths) => paths,
            Err(err) => {
                self.kernel
                    .bus
                    .print_warning(format!("Could not prepare recent thumbnail cache: {err}"));
                return;
            }
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        let session = &mut self.kernel.session;
        let capture = renderer.capture_png(
            &paths.temp_path,
            RECENT_THUMBNAIL_WIDTH,
            RECENT_THUMBNAIL_HEIGHT,
            &mut session.camera,
            &mut session.registry,
            &session.settings,
            &session.named_palette,
            &session.palette,
            session.clear_color,
        );

        if let Err(err) = capture.and_then(|_| {
            paths
                .commit()
                .map_err(|err| patinae_scene::ViewerError::capture_error(err.to_string()))
        }) {
            paths.cleanup_temp();
            self.kernel
                .bus
                .print_warning(format!("Could not save recent thumbnail: {err}"));
            return;
        }

        self.recent_files.refresh(app);
    }

    fn sync_empty_mode(&mut self, app: &AppWindow) {
        let empty = self.kernel.session.registry.is_empty();
        let layout = app.global::<LayoutState>();
        layout.set_empty_mode(empty);

        if should_apply_standard_panel_preset(self.last_empty_mode, empty) {
            apply_standard_panel_preset(&layout);
        }

        self.last_empty_mode = empty;
    }

    pub(crate) fn sync_plugins(&mut self, app: &AppWindow) {
        if self.plugins.plugin_count() == 0 {
            if self.last_plugin_sync_key != Some(0) {
                self.plugin_bridge.sync(Vec::new(), app);
                self.last_plugin_sync_key = Some(0);
            }
            return;
        }

        let sync_key = self.plugin_sync_key();
        if self.last_plugin_sync_key == Some(sync_key) {
            return;
        }

        let gpu_device = self.renderer.as_ref().map(|r| r.gpu_device().as_ref());
        let gpu_queue = self.renderer.as_ref().map(|r| r.gpu_queue().as_ref());
        let scene_generation = self
            .kernel
            .session
            .registry
            .generation()
            .wrapping_add(self.kernel.session.selections.generation().rotate_left(1))
            .wrapping_add(self.kernel.session.movie.generation().rotate_left(2));
        let shared = SharedContext {
            registry: &self.kernel.session.registry,
            camera: &self.kernel.session.camera,
            selections: &self.kernel.session.selections,
            named_palette: &self.kernel.session.named_palette,
            movie: &self.kernel.session.movie,
            settings: &self.kernel.session.settings,
            clear_color: self.kernel.session.clear_color,
            gpu_device,
            gpu_queue,
            scene_generation,
            viewport_image: self.kernel.session.viewport_image.as_ref(),
            command_names: &self.command_names_cache,
            command_registry: self.kernel.executor.registry(),
            setting_names: &self.setting_names_cache,
            dynamic_settings: Some(self.kernel.executor.dynamic_settings()),
        };
        let frames = self.plugins.panel_frames(&shared, sync_key);
        self.plugin_bridge.sync(frames, app);
        self.last_plugin_sync_key = Some(sync_key);
    }

    pub(crate) fn notify_short(&mut self, message: impl Into<String>) {
        self.transient_notification = Some((
            message.into(),
            Instant::now() + Duration::from_secs(TRANSIENT_NOTIFICATION_SECS),
        ));
    }

    fn sync_notifications(&mut self, app: &AppWindow) {
        let mut messages = self.kernel.task_notification_messages();
        messages.extend(self.plugins.notification_messages().iter().cloned());
        if messages.is_empty() {
            if let Some((message, expires_at)) = &self.transient_notification {
                if Instant::now() <= *expires_at {
                    messages.push(message.clone());
                } else {
                    self.transient_notification = None;
                }
            }
        }

        let state = app.global::<NotificationState>();
        state.set_visible(!messages.is_empty());
        state.set_text(messages.join("\n").into());
    }

    fn refresh_command_names_cache(&mut self) {
        self.command_names_cache = self
            .kernel
            .executor
            .registry()
            .names()
            .map(|s| s.to_string())
            .collect();
    }

    fn plugin_sync_key(&self) -> u64 {
        let mut key = self.plugins.panel_ui_generation();
        key = mix_generation(key, self.plugin_scene_generation());
        key = mix_generation(key, self.kernel.command_generation());
        key = mix_generation(
            key,
            viewport_image_signature(self.kernel.session.viewport_image.as_ref()),
        );
        key
    }

    fn plugin_scene_generation(&self) -> u64 {
        self.kernel
            .session
            .registry
            .generation()
            .wrapping_add(self.kernel.session.selections.generation().rotate_left(1))
            .wrapping_add(self.kernel.session.movie.generation().rotate_left(2))
    }

    fn run_startup_actions(&mut self, app: &AppWindow, viewport_size: (u32, u32)) {
        while let Some(action) = self.startup_actions.pop_front() {
            match action {
                StartupAction::Warning(message) => {
                    self.kernel.bus.print_warning(message);
                }
                StartupAction::RunPatinaerc(path) => {
                    app.global::<crate::ReplState>().set_busy(true);
                    let path_text = path.to_string_lossy();
                    let command = format!("run {}", quote_command_arg(path_text.as_ref()));
                    let rc: Option<&mut dyn CaptureRenderer> = self
                        .renderer
                        .as_mut()
                        .map(|r| r as &mut dyn CaptureRenderer);
                    let _ = self.kernel.execute_command_warning_on_error(
                        &command,
                        false,
                        rc,
                        viewport_size,
                    );
                }
                StartupAction::RouteArgvFile(path) => {
                    app.global::<crate::ReplState>().set_busy(true);
                    enqueue_file_action(&mut self.kernel, &path, NativeFileSource::StartupArgv);
                }
            }
        }
    }

    /// Refresh the `PATINAE_PERF=1` HUD overlay. Throttled to ~12 Hz so
    /// the Slint property writes don't fight render-loop priority.
    fn update_perf_hud(&self, vp: &ViewportState, gpu_stats: Option<&patinae_render::FrameStats>) {
        const REFRESH_EVERY: u32 = 10;
        let n = self.perf_update_counter.get().wrapping_add(1);
        self.perf_update_counter.set(n);
        if !n.is_multiple_of(REFRESH_EVERY) {
            return;
        }
        let history = self.perf_history.borrow();
        if history.is_empty() {
            return;
        }
        let avg_ms = history.avg_ms();
        let low_1 = history.low_1pct_ms();
        let low_01 = history.low_0_1pct_ms();
        vp.set_perf_visible(true);
        // Labels ("avg" / "1% low" / "0.1% low") live in the HUD; feed values only.
        vp.set_perf_avg_text(format!("{:.2} ms", avg_ms).into());
        vp.set_perf_low_1pct_text(format!("{:.2} ms", low_1).into());
        vp.set_perf_low_0_1pct_text(format!("{:.2} ms", low_01).into());
        if let Some(renderer) = self.renderer.as_ref() {
            let profile = renderer.memory_profile();
            let memory = renderer.memory_snapshot();
            vp.set_perf_memory_profile_label(memory_profile_hud_label(profile).into());
            vp.set_perf_memory_total_text(
                memory_total_hud_text(memory.total_capacity_bytes(), profile.budget_bytes()).into(),
            );
            vp.set_perf_memory_live_text(format_hud_mib(memory.total_live_bytes()).into());
            vp.set_perf_memory_fill(memory_budget_fill(
                memory.total_capacity_bytes(),
                profile.budget_bytes(),
            ));
        }
        if let Some(s) = gpu_stats {
            // Sentinel `-1.0` means the pass didn't run this frame.
            let fmt = |label: &str, ms: f32| -> String {
                if ms < 0.0 {
                    String::new()
                } else {
                    format!("{label} {ms:.2}")
                }
            };
            let mut parts: Vec<String> = Vec::with_capacity(6);
            for (label, ms) in [
                ("cull", s.gpu_cull_ms),
                ("shadow", s.gpu_shadow_ms),
                ("atlas_ao", s.gpu_atlas_ao_ms),
                ("opaque", s.gpu_opaque_ms),
                ("mark", s.gpu_marking_ms),
                ("translucent", s.gpu_translucent_ms),
            ] {
                let s = fmt(label, ms);
                if !s.is_empty() {
                    parts.push(s);
                }
            }
            vp.set_perf_gpu_text(parts.join(" · ").into());
        }
    }

    // --- Picking ---

    /// Clear hover indicators and reset hover state.
    fn clear_hover_indicators(&mut self) {
        self.kernel.session.clear_hover();
        self.kernel.viewport.hover_hit = None;
    }

    /// Process hover indicators: ray-cast each frame when no button is pressed.
    fn process_hover(&mut self, vw: u32, vh: u32, sf: f32) {
        if self.kernel.viewport.input.any_button_pressed() {
            return;
        }
        if self.renderer.is_none() {
            return;
        }
        if !self.cursor_known.get() {
            return;
        }

        // Cursor position from winit (physical pixels, window-relative).
        // Convert to viewport-relative by subtracting sidebar.
        let (wx, wy) = self.cursor_pos.get();
        let mx = wx - SIDEBAR_WIDTH_LP * sf;
        let my = wy;
        if mx < 0.0 || mx > vw as f32 || my < 0.0 || my > vh as f32 {
            // Cursor is outside the viewport (over sidebar, dock, etc.)
            if self.kernel.viewport.hover_hit.is_some() {
                self.clear_hover_indicators();
            }
            return;
        }

        // Async hover pick: returns `None` when no fresh result is ready
        // this frame — keep the existing hover indicator until the next pick
        // resolves. `Some(_)` carries a fresh hit/miss to apply.
        let resolved = self
            .renderer
            .as_mut()
            .and_then(|r| r.poll_hover_pick(&self.kernel.session, mx as u32, my as u32));
        let Some(new_hit) = resolved else {
            return;
        };

        // Change detection — skip GPU work if nothing changed.
        let changed = match (&self.kernel.viewport.hover_hit, &new_hit) {
            (None, None) => false,
            (Some(_), None) | (None, Some(_)) => true,
            (Some(a), Some(b)) => a.object_name != b.object_name || a.atom_index != b.atom_index,
        };
        if !changed {
            return;
        }

        let mode = self.kernel.session.settings.ui.mouse_selection_mode as i32;

        // Resolve hit → hover target (single global, not per-molecule).
        if let Some(hit) = new_hit.as_ref() {
            if let Some(mol_obj) = self.kernel.session.registry.get_molecule(&hit.object_name) {
                let mol = mol_obj.molecule();
                let sel = expand_pick_to_selection(hit, mode, mol);
                self.kernel.session.set_hover(patinae_scene::HoverTarget {
                    object: hit.object_name.clone(),
                    selection: sel,
                });
            } else {
                self.kernel.session.clear_hover();
            }
        } else {
            self.kernel.session.clear_hover();
        }

        self.kernel.viewport.hover_hit = new_hit;
    }

    /// Process a click: GPU pick and update the `sele` named selection.
    fn process_click(&mut self, click_pos: (f32, f32), viewport: (u32, u32)) {
        // Ignore clicks outside the 3D viewport. The popover-dismiss backdrop
        // (layout.slint) forwards pointer-down/up to ViewportState so a drag
        // through the backdrop rotates the camera; a plain dismiss-click
        // would otherwise be seen here as a viewport miss and run
        // `deselect sele`, wiping the user's selection.
        let (vw, vh) = viewport;
        if click_pos.0 < 0.0
            || click_pos.0 >= vw as f32
            || click_pos.1 < 0.0
            || click_pos.1 >= vh as f32
        {
            return;
        }

        if self.dismiss_viewport_image() {
            return;
        }

        let hit = self.renderer.as_mut().and_then(|r| {
            r.pick_at_click(&self.kernel.session, click_pos.0 as u32, click_pos.1 as u32)
        });

        let cmd = if let Some(ref hit) = hit {
            if let Some(mol_obj) = self.kernel.session.registry.get_molecule(&hit.object_name) {
                let mol = mol_obj.molecule();
                let mode = self.kernel.session.settings.ui.mouse_selection_mode as i32;

                pick_expression_for_hit(hit, mode, mol).and_then(|expr| {
                    let overlaps_sele =
                        self.kernel
                            .session
                            .selections
                            .get("sele")
                            .is_some_and(|entry| {
                                let sel = expand_pick_to_selection(hit, mode, mol);
                                patinae_select::select(mol, &entry.expression)
                                    .map(|sele| sel.intersection(&sele).any())
                                    .unwrap_or(false)
                            });
                    let has_sele = self.kernel.session.selections.contains("sele");
                    build_sele_command(&expr, overlaps_sele, has_sele)
                })
            } else {
                None
            }
        } else {
            // Miss — clear selection if one exists.
            if self.kernel.session.selections.contains("sele") {
                Some("deselect sele".to_string())
            } else {
                None
            }
        };

        if let Some(cmd) = cmd {
            self.kernel.bus.execute_command(cmd);
        }
    }

    // --- Helpers ---

    fn dismiss_viewport_image(&mut self) -> bool {
        let had_cpu_image = self.kernel.session.viewport_image.take().is_some();
        let had_gpu_image = self
            .renderer
            .as_mut()
            .is_some_and(|renderer| renderer.clear_viewport_gpu_image());
        let had_image = had_cpu_image || had_gpu_image;
        if had_image {
            self.kernel.session.clear_hover();
            self.kernel.viewport.hover_hit = None;
        }
        had_image
    }

    fn viewport_size(app: &AppWindow) -> (u32, u32) {
        let ws = app.window().size();
        let sf = app.window().scale_factor();
        let layout = app.global::<LayoutState>();
        viewport_size_from_parts(
            ws.width,
            ws.height,
            sf,
            layout.get_effective_right_dock_width(),
            layout.get_effective_bottom_dock_height(),
        )
    }

    fn pointer_snapshot(
        &self,
        vp: &ViewportState,
        vw: u32,
        vh: u32,
        sf: f32,
        repl_visible: bool,
        modal_visible: bool,
    ) -> PointerSnapshot {
        let sidebar = SIDEBAR_WIDTH_LP * sf;
        let raw_cursor_known = self.cursor_known.get();
        let raw_press_known = self.raw_press_known.get();
        let mouse_logical = if raw_cursor_known {
            let (wx, wy) = self.cursor_pos.get();
            ((wx - sidebar) / sf, wy / sf)
        } else {
            (vp.get_mouse_x(), vp.get_mouse_y())
        };
        let press_logical = if raw_press_known {
            let (px, py) = self.raw_press_pos.get();
            ((px - sidebar) / sf, py / sf)
        } else {
            (vp.get_press_x(), vp.get_press_y())
        };

        let press_phys = (press_logical.0 * sf, press_logical.1 * sf);
        let press_inside = press_phys.0 >= 0.0
            && press_phys.0 < vw as f32
            && press_phys.1 >= 0.0
            && press_phys.1 < vh as f32
            && !modal_visible
            && !Self::point_over_repl_pill(press_logical, vw, vh, sf, repl_visible);
        let raw_buttons = self.raw_mouse_buttons.get();
        let slint_buttons = (
            vp.get_left_pressed(),
            vp.get_middle_pressed(),
            vp.get_right_pressed(),
        );
        let raw_buttons_authoritative =
            raw_press_known || raw_cursor_known || raw_buttons != slint_buttons;
        let (left, middle, right) = if raw_buttons_authoritative {
            raw_buttons
        } else {
            slint_buttons
        };

        PointerSnapshot {
            mouse_logical,
            press_logical,
            left_pressed: left && press_inside,
            middle_pressed: middle && press_inside,
            right_pressed: right && press_inside,
            suppress_click: modal_visible || vp.get_suppress_click(),
        }
    }

    fn point_over_repl_pill(
        point_logical: (f32, f32),
        vw: u32,
        vh: u32,
        sf: f32,
        repl_visible: bool,
    ) -> bool {
        if !repl_visible {
            return false;
        }

        let viewport_w_lp = vw as f32 / sf;
        let viewport_h_lp = vh as f32 / sf;
        let pill_x = (viewport_w_lp - REPL_PILL_WIDTH_LP) * 0.5;
        let pill_y = viewport_h_lp - REPL_PILL_BOTTOM_LP - REPL_PILL_HEIGHT_LP;

        point_logical.0 >= pill_x
            && point_logical.0 < pill_x + REPL_PILL_WIDTH_LP
            && point_logical.1 >= pill_y
            && point_logical.1 < pill_y + REPL_PILL_HEIGHT_LP
    }
}

fn viewport_size_from_parts(
    window_width_px: u32,
    window_height_px: u32,
    scale_factor: f32,
    right_dock_width_lp: f32,
    bottom_dock_height_lp: f32,
) -> (u32, u32) {
    let scale_factor = scale_factor.max(0.0);
    let sidebar = SIDEBAR_WIDTH_LP * scale_factor;
    let right = right_dock_width_lp * scale_factor;
    let bottom = bottom_dock_height_lp * scale_factor;
    let width = (window_width_px as f32 - sidebar - right).max(1.0) as u32;
    let height = (window_height_px as f32 - bottom).max(1.0) as u32;
    (width, height)
}

fn viewport_label_from_projected(
    label: &ProjectedAnnotationLabel,
    scale_factor: f32,
) -> ViewportLabel {
    let scale_factor = scale_factor.max(f32::EPSILON);
    let (anchor_x, anchor_y) = label.alignment.anchor_factors();
    ViewportLabel {
        x: label.x / scale_factor,
        y: label.y / scale_factor,
        text: label.text.clone().into(),
        color: slint::Color::from_argb_f32(
            label.color[3],
            label.color[0],
            label.color[1],
            label.color[2],
        ),
        // Label sizes are device-independent UI pixels. Slint applies the
        // window scale factor when it rasterizes the glyphs.
        size: label.size,
        anchor_x,
        anchor_y,
        alignment: label.alignment.as_str().into(),
        owner_id: label.owner_id.get().to_string().into(),
        insertion_ordinal: i32::try_from(label.insertion_ordinal).unwrap_or(i32::MAX),
        display_order: label.display_order.to_string().into(),
    }
}

fn startup_actions(patinaerc: PatinaercPath, argv_files: Vec<PathBuf>) -> VecDeque<StartupAction> {
    let mut actions = VecDeque::new();

    match patinaerc.path.try_exists() {
        Ok(true) | Err(_) => actions.push_back(StartupAction::RunPatinaerc(patinaerc.path)),
        Ok(false) if patinaerc.source == PatinaercSource::Env => {
            actions.push_back(StartupAction::Warning(format!(
                "PATINAERC startup rc script not found: {}",
                patinaerc.path.display()
            )));
        }
        Ok(false) => {}
    }

    for path in argv_files {
        actions.push_back(StartupAction::RouteArgvFile(path));
    }

    actions
}

fn recent_open_command_for_existing_path(path: &Path) -> String {
    let path_text = path.to_string_lossy();
    format!("load {}", quote_command_arg(path_text.as_ref()))
}

fn should_apply_standard_panel_preset(was_empty: bool, is_empty: bool) -> bool {
    was_empty && !is_empty
}

fn apply_standard_panel_preset(layout: &LayoutState) {
    layout.set_objects_visible(true);
    layout.set_selections_visible(true);
    layout.set_repl_visible(true);
    layout.set_sequence_visible(false);
    layout.set_movie_visible(false);
    layout.set_bottom_active_tab(0);
}

#[cfg(any(not(target_os = "windows"), test))]
fn patinae_wgpu_required_limits(
    adapter_limits: &slint::wgpu_29::wgpu::Limits,
    memory_policy: RenderMemoryPolicy,
) -> slint::wgpu_29::wgpu::Limits {
    required_limits_for_memory_policy(adapter_limits, memory_policy)
}

fn patinae_wgpu_backends(
    env_backends: Option<slint::wgpu_29::wgpu::Backends>,
    is_windows: bool,
) -> slint::wgpu_29::wgpu::Backends {
    let backends = if is_windows {
        env_backends.unwrap_or(slint::wgpu_29::wgpu::Backends::DX12)
    } else {
        env_backends.unwrap_or_default()
    };
    backends & !slint::wgpu_29::wgpu::Backends::GL
}

fn patinae_wgpu_settings() -> slint::wgpu_29::WGPUSettings {
    let mut settings = slint::wgpu_29::WGPUSettings::default();
    settings.power_preference = slint::wgpu_29::wgpu::PowerPreference::HighPerformance;
    settings.backends = patinae_wgpu_backends(
        slint::wgpu_29::wgpu::Backends::from_env(),
        cfg!(target_os = "windows"),
    );
    settings
        .device_required_limits
        .max_storage_buffers_per_shader_stage = RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE;
    settings
}

fn patinae_wgpu_instance(
    settings: &slint::wgpu_29::WGPUSettings,
) -> slint::wgpu_29::wgpu::Instance {
    pollster::block_on(
        slint::wgpu_29::wgpu::util::new_instance_with_webgpu_detection(
            slint::wgpu_29::wgpu::InstanceDescriptor {
                // Match Slint's femtovg-wgpu automatic path, which avoids GL.
                backends: settings.backends,
                flags: settings.instance_flags,
                backend_options: settings.backend_options.clone(),
                memory_budget_thresholds: settings.instance_memory_budget_thresholds,
                display: None,
            },
        ),
    )
}

#[cfg(target_os = "windows")]
fn patinae_wgpu_configuration(
) -> Result<slint::wgpu_29::WGPUConfiguration, Box<dyn std::error::Error>> {
    let mut settings = patinae_wgpu_settings();
    // Slint's automatic path selects the adapter after creating the window
    // surface. On Windows this avoids choosing a headless-incompatible adapter
    // before winit has a surface, while still giving Patinae native compute and
    // storage-buffer limits.
    log_windows_wgpu_limits("wgpu Windows configuration", &settings);

    if std::env::var_os("SLINT_WGPU_CPU").is_some() {
        return Err(cpu_wgpu_adapter_error().into());
    }

    if !patinae_windows_has_hardware_adapter(&settings) {
        return Err(no_hardware_wgpu_adapter_error(settings.backends, None).into());
    }

    log::info!("wgpu Windows automatic configuration: hardware adapter preflight succeeded");
    Ok(slint::wgpu_29::WGPUConfiguration::Automatic(settings))
}

#[cfg(target_os = "windows")]
fn log_windows_wgpu_limits(prefix: &str, settings: &slint::wgpu_29::WGPUSettings) {
    log::info!(
        "{prefix}: backends={:?} requested max_storage_buffer_binding_size={} \
         max_buffer_size={} max_storage_buffers_per_shader_stage={}",
        settings.backends,
        settings
            .device_required_limits
            .max_storage_buffer_binding_size,
        settings.device_required_limits.max_buffer_size,
        settings
            .device_required_limits
            .max_storage_buffers_per_shader_stage,
    );
}

#[cfg(target_os = "windows")]
fn patinae_windows_has_hardware_adapter(settings: &slint::wgpu_29::WGPUSettings) -> bool {
    let instance = patinae_wgpu_instance(settings);
    let adapters = pollster::block_on(instance.enumerate_adapters(settings.backends));
    if adapters.is_empty() {
        log::warn!(
            "wgpu Windows adapter preflight: no adapters for {:?}",
            settings.backends
        );
        return false;
    }

    let mut has_hardware = false;
    for adapter in &adapters {
        let info = adapter.get_info();
        log::info!(
            "wgpu Windows adapter preflight: name={} backend={:?} type={:?}",
            info.name,
            info.backend,
            info.device_type
        );
        has_hardware |= info.device_type != slint::wgpu_29::wgpu::DeviceType::Cpu;
    }
    has_hardware
}

#[cfg(not(target_os = "windows"))]
fn patinae_wgpu_configuration(
) -> Result<slint::wgpu_29::WGPUConfiguration, Box<dyn std::error::Error>> {
    let mut settings = patinae_wgpu_settings();

    let instance = patinae_wgpu_instance(&settings);

    let adapter = pollster::block_on(async {
        match slint::wgpu_29::wgpu::util::initialize_adapter_from_env(&instance, None).await {
            Ok(adapter) => Ok(adapter),
            Err(_) => {
                instance
                    .request_adapter(&slint::wgpu_29::wgpu::RequestAdapterOptions {
                        power_preference: settings.power_preference,
                        force_fallback_adapter: false,
                        compatible_surface: None,
                    })
                    .await
            }
        }
    })?;

    let adapter_limits = adapter.limits();
    let adapter_features = adapter.features();
    let adapter_info = adapter.get_info();
    if adapter_info.device_type == slint::wgpu_29::wgpu::DeviceType::Cpu {
        return Err(no_hardware_wgpu_adapter_error(settings.backends, Some(&adapter_info)).into());
    }
    log_selected_gpu(&adapter_info);
    let memory_policy = render_memory_policy_for_device(
        Some(&adapter_info),
        &adapter_limits,
        "PATINAE_RENDER_MEMORY_PROFILE",
        false,
    );
    settings.device_memory_hints = memory_policy.wgpu_memory_hints();
    let binding_array_features = slint::wgpu_29::wgpu::Features::BUFFER_BINDING_ARRAY
        | slint::wgpu_29::wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY;
    let requested_binding_array_features = adapter_features & binding_array_features;
    let requested_timestamp_query_feature =
        adapter_features & slint::wgpu_29::wgpu::Features::TIMESTAMP_QUERY;
    settings.device_required_features |=
        requested_binding_array_features | requested_timestamp_query_feature;
    let required_limits = patinae_wgpu_required_limits(&adapter_limits, memory_policy);
    log::info!(
        "wgpu limits: adapter max_storage_buffer_binding_size={} max_buffer_size={}; \
         requested max_storage_buffer_binding_size={} max_buffer_size={} \
         max_storage_buffers_per_shader_stage={}",
        adapter_limits.max_storage_buffer_binding_size,
        adapter_limits.max_buffer_size,
        required_limits.max_storage_buffer_binding_size,
        required_limits.max_buffer_size,
        required_limits.max_storage_buffers_per_shader_stage,
    );
    log::info!(
        "wgpu optional features: buffer_binding_array={} storage_resource_binding_array={} timestamp_query={}",
        requested_binding_array_features
            .contains(slint::wgpu_29::wgpu::Features::BUFFER_BINDING_ARRAY),
        requested_binding_array_features
            .contains(slint::wgpu_29::wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY),
        requested_timestamp_query_feature.contains(slint::wgpu_29::wgpu::Features::TIMESTAMP_QUERY),
    );
    log::info!(
        "render memory profile: profile={} budget={:?} memory_hints={:?}",
        memory_policy.profile,
        memory_policy.budget_bytes,
        settings.device_memory_hints
    );

    let (device, queue) = pollster::block_on(adapter.request_device(
        &slint::wgpu_29::wgpu::DeviceDescriptor {
            label: settings.device_label.as_deref(),
            required_features: settings.device_required_features,
            required_limits,
            experimental_features: settings.device_experimental_features,
            memory_hints: settings.device_memory_hints,
            trace: slint::wgpu_29::wgpu::Trace::default(),
        },
    ))?;

    Ok(slint::wgpu_29::WGPUConfiguration::Manual {
        instance,
        adapter,
        device,
        queue,
    })
}

// ---------------------------------------------------------------------------
// run() — entry point
// ---------------------------------------------------------------------------

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let wgpu_configuration = match patinae_wgpu_configuration() {
        Ok(configuration) => configuration,
        Err(err) => return abort_startup_for_renderer_error(err),
    };
    let selector = slint::BackendSelector::new().require_wgpu_29(wgpu_configuration);

    #[cfg(target_os = "macos")]
    let selector = selector.with_winit_window_attributes_hook(|attrs| {
        crate::macos::configure_titlebar_attributes(attrs)
    });

    if let Err(err) = selector.select() {
        return abort_startup_for_renderer_error(err);
    }

    let mut app_state = App::new();
    app_state.startup_actions = startup_actions(
        patinae_settings::paths::patinaerc_path(),
        std::env::args_os().skip(1).map(PathBuf::from).collect(),
    );
    let app = Rc::new(RefCell::new(app_state));

    let window = match AppWindow::new() {
        Ok(window) => window,
        Err(err) => return abort_startup_for_renderer_error(err),
    };

    #[cfg(target_os = "macos")]
    let display_link_heartbeat: Rc<
        RefCell<Option<crate::cv_display_link::DisplayLinkHeartbeat>>,
    > = Rc::new(RefCell::new(None));

    {
        #[cfg(target_os = "macos")]
        let display_link_heartbeat_for_close = display_link_heartbeat.clone();
        window.window().on_close_requested(move || {
            #[cfg(target_os = "macos")]
            display_link_heartbeat_for_close.borrow_mut().take();
            request_event_loop_quit(slint::quit_event_loop);
            slint::CloseRequestResponse::HideWindow
        });
    }

    #[cfg(not(target_os = "macos"))]
    window.global::<LayoutState>().set_mod_key("Ctrl".into());

    #[cfg(target_os = "macos")]
    {
        let dark = crate::macos::is_system_dark_mode();
        let mode = if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        app.borrow_mut().kernel.session.settings.ui.theme = mode;
        // sync_theme on first frame will set Slint dark-mode + palette + bg
    }

    // Push fixed solid-color swatches once (theme-independent)
    crate::bridges::theme::push_solid_swatches(&window.global::<Theme>());
    app.borrow().recent_files.attach(&window);

    // Winit event hook: ModifiersChanged (all platforms), Focused, PinchGesture (macOS)
    {
        use slint::winit_030::EventResult;
        use slint::winit_030::WinitWindowAccessor;

        #[cfg(target_os = "macos")]
        let pinch_accum = app.borrow().pinch_accumulator.clone();
        let display_recovery = app.borrow().display_recovery.clone();
        let winit_mods = app.borrow().winit_modifiers.clone();
        let focus_lost = app.borrow().window_focus_lost.clone();
        let cursor = app.borrow().cursor_pos.clone();
        let cursor_known = app.borrow().cursor_known.clone();
        let buttons = app.borrow().raw_mouse_buttons.clone();
        let press_pos = app.borrow().raw_press_pos.clone();
        let press_known = app.borrow().raw_press_known.clone();
        let app_for_drop = app.clone();
        let app_for_keys = app.clone();
        window
            .window()
            .on_winit_window_event(move |slint_window, event| {
                use slint::winit_030::winit::event::WindowEvent;
                match event {
                    #[cfg(target_os = "macos")]
                    WindowEvent::PinchGesture { delta, .. } => {
                        pinch_accum.set(pinch_accum.get() + delta);
                        return EventResult::PreventDefault;
                    }
                    WindowEvent::CursorMoved { position, .. } => {
                        cursor.set((position.x as f32, position.y as f32));
                        cursor_known.set(true);
                    }
                    WindowEvent::MouseInput { state, button, .. } => {
                        use slint::winit_030::winit::event::{ElementState, MouseButton};

                        let pressed = *state == ElementState::Pressed;
                        let mut current = buttons.get();
                        match *button {
                            MouseButton::Left => current.0 = pressed,
                            MouseButton::Middle => current.1 = pressed,
                            MouseButton::Right => current.2 = pressed,
                            _ => {}
                        }
                        if pressed {
                            if cursor_known.get() {
                                press_pos.set(cursor.get());
                                press_known.set(true);
                            } else {
                                press_known.set(false);
                            }
                        } else if !current.0 && !current.1 && !current.2 {
                            press_known.set(false);
                        }
                        buttons.set(current);
                    }
                    WindowEvent::DroppedFile(path) => {
                        app_for_drop.borrow_mut().route_dropped_file(path);
                        slint_window.request_redraw();
                    }
                    WindowEvent::Resized(size) => {
                        log_window_event(
                            slint_window,
                            "resized",
                            format_args!("size={}x{}", size.width, size.height),
                        );
                        mark_display_transition_and_redraw(&display_recovery, slint_window);
                    }
                    WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                        log_window_event(
                            slint_window,
                            "scale-factor-changed",
                            format_args!("scale_factor={scale_factor:.3}"),
                        );
                        mark_display_transition_and_redraw(&display_recovery, slint_window);
                    }
                    WindowEvent::Occluded(occluded) => {
                        log_window_event(
                            slint_window,
                            "occluded",
                            format_args!("occluded={occluded}"),
                        );
                        if !occluded {
                            mark_display_transition_and_redraw(&display_recovery, slint_window);
                        }
                    }
                    WindowEvent::ModifiersChanged(modifiers) => {
                        let state = modifiers.state();
                        winit_mods.set((
                            state.shift_key(),
                            state.control_key(),
                            state.alt_key(),
                            state.super_key(),
                        ));
                    }
                    WindowEvent::KeyboardInput { event, .. } => {
                        use slint::winit_030::winit::event::ElementState;
                        use slint::winit_030::winit::keyboard::PhysicalKey;

                        if let PhysicalKey::Code(code) = event.physical_key {
                            if let Some(key) = map_winit_key(code) {
                                let (shift, ctrl, alt, super_key) = winit_mods.get();

                                if event.state == ElementState::Pressed {
                                    let binding = KeyBinding {
                                        key,
                                        ctrl: ctrl || super_key,
                                        shift,
                                        alt,
                                    };
                                    let mut a = app_for_keys.borrow_mut();
                                    let mut bus = std::mem::take(&mut a.kernel.bus);
                                    let handled = a.plugins.handle_hotkey(binding, &mut bus);
                                    a.kernel.bus = bus;
                                    if handled {
                                        return EventResult::PreventDefault;
                                    }
                                }

                                if let Some(text) =
                                    normalized_slint_shortcut_text(key, shift, ctrl, alt, super_key)
                                {
                                    let event = match event.state {
                                        ElementState::Pressed if event.repeat => {
                                            slint::platform::WindowEvent::KeyPressRepeated {
                                                text: text.into(),
                                            }
                                        }
                                        ElementState::Pressed => {
                                            slint::platform::WindowEvent::KeyPressed {
                                                text: text.into(),
                                            }
                                        }
                                        ElementState::Released => {
                                            slint::platform::WindowEvent::KeyReleased {
                                                text: text.into(),
                                            }
                                        }
                                    };
                                    slint_window.dispatch_event(event);
                                    return EventResult::PreventDefault;
                                }
                            }
                        }
                    }
                    WindowEvent::Focused(true) => {
                        log_window_event(slint_window, "focused", format_args!("focused=true"));
                        mark_display_transition_and_redraw(&display_recovery, slint_window);
                    }
                    WindowEvent::Focused(false) => {
                        log_window_event(slint_window, "focused", format_args!("focused=false"));
                        winit_mods.set((false, false, false, false));
                        buttons.set((false, false, false));
                        cursor_known.set(false);
                        press_known.set(false);
                        focus_lost.set(true);
                    }
                    _ => {}
                }
                EventResult::Propagate
            });
    }

    // Attach viewport and objects bridge models to Slint globals + wire callbacks
    app.borrow().viewport.attach(&window);
    app.borrow().objects.attach(&window);
    crate::bridges::objects::setup_callbacks(app.clone(), &window);

    // Attach sequence bridge + wire callbacks
    app.borrow().sequence.attach(&window);
    crate::bridges::sequence::setup_callbacks(app.clone(), &window);

    // Attach movie bridge + wire callbacks
    app.borrow().movie.attach(&window);
    crate::bridges::movie::setup_callbacks(app.clone(), &window);

    // Attach REPL bridge + wire callbacks
    app.borrow().repl.attach(&window);
    crate::bridges::repl::setup_callbacks(app.clone(), &window);

    // Attach plugin panel bridge + wire callbacks
    app.borrow().plugin_bridge.attach(&window);
    crate::bridges::plugins::setup_callbacks(app.clone(), &window);

    // Wire layout panel show/hide callbacks
    crate::bridges::layout::setup_callbacks(app.clone(), &window);

    // Wire native menu and fetch dialog callbacks
    crate::native_menu::setup_callbacks(app.clone(), &window);

    // Theme toggle callback — sends `set theme` through the message bus
    {
        let app_ref = app.clone();
        window.global::<Theme>().on_toggle_theme(move || {
            let mut a = app_ref.borrow_mut();
            let new_theme = if a.kernel.session.settings.ui.theme == ThemeMode::Dark {
                "light"
            } else {
                "dark"
            };
            a.kernel
                .bus
                .execute_command_silent(format!("set theme, {new_theme}"));
        });
    }

    let app_rc = app.clone();
    let window_weak = window.as_weak();

    let timing_callbacks = std::env::var("PATINAE_TIMING").is_ok();
    let before_end: std::rc::Rc<std::cell::Cell<Option<std::time::Instant>>> =
        std::rc::Rc::new(std::cell::Cell::new(None));
    let before_end_clone = before_end.clone();
    #[cfg(target_os = "macos")]
    let display_link_heartbeat_for_notifier = display_link_heartbeat.clone();
    window
        .window()
        .set_rendering_notifier(move |rs, api| match rs {
            slint::RenderingState::RenderingSetup => {
                if let Some(w) = window_weak.upgrade() {
                    app_rc.borrow_mut().setup_renderer(api, &w);
                    log_window_event(w.window(), "rendering-setup", format_args!("wgpu-ready"));
                    request_display_recovery_redraw(w.window());
                    #[cfg(target_os = "macos")]
                    start_display_link_heartbeat(
                        w.as_weak(),
                        display_link_heartbeat_for_notifier.clone(),
                    );
                }
            }
            slint::RenderingState::BeforeRendering => {
                if let Some(w) = window_weak.upgrade() {
                    app_rc.borrow_mut().render_frame(&w);
                    schedule_pending_save_file_requests(app_rc.clone(), window_weak.clone());
                    w.window().request_redraw();
                    if timing_callbacks {
                        before_end_clone.set(Some(std::time::Instant::now()));
                    }
                }
            }
            slint::RenderingState::AfterRendering if timing_callbacks => {
                if let Some(t0) = before_end_clone.get() {
                    let dt = t0.elapsed().as_secs_f32() * 1000.0;
                    eprintln!("[patinae] slint scene-graph render: {dt:.2} ms");
                }
            }
            slint::RenderingState::RenderingTeardown => {
                #[cfg(target_os = "macos")]
                display_link_heartbeat_for_notifier.borrow_mut().take();
                app_rc.borrow_mut().teardown_renderer();
            }
            _ => {}
        })?;

    window.run()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn start_display_link_heartbeat(
    window: slint::Weak<AppWindow>,
    heartbeat: Rc<RefCell<Option<crate::cv_display_link::DisplayLinkHeartbeat>>>,
) {
    if heartbeat.borrow().is_some() {
        return;
    }

    match crate::cv_display_link::DisplayLinkHeartbeat::start(window) {
        Ok(link) => {
            *heartbeat.borrow_mut() = Some(link);
            log::info!("Started macOS CVDisplayLink render heartbeat");
        }
        Err(err) => {
            log::warn!("Failed to start macOS CVDisplayLink render heartbeat: {err}");
        }
    }
}

const STARTUP_RENDERER_ALERT_TITLE: &str = "Patinae Renderer Unavailable";

fn abort_startup_for_renderer_error(
    err: impl std::fmt::Display,
) -> Result<(), Box<dyn std::error::Error>> {
    let message = startup_renderer_error_message(&err.to_string());
    startup_alert::show_error(STARTUP_RENDERER_ALERT_TITLE, &message);
    Err(std::io::Error::other(message).into())
}

fn startup_renderer_error_message(reason: &str) -> String {
    format!(
        "Patinae could not start the hardware-accelerated WGPU renderer required \
         for the interactive viewport.\n\n{reason}\n\nEnable VM 3D acceleration, \
         install a Vulkan/DirectX 12-capable graphics driver, or run Patinae on \
         a machine with supported GPU acceleration. Patinae will now exit."
    )
}

fn no_hardware_wgpu_adapter_error(
    backends: slint::wgpu_29::wgpu::Backends,
    selected_cpu_adapter: Option<&slint::wgpu_29::wgpu::AdapterInfo>,
) -> std::io::Error {
    let selected = selected_cpu_adapter
        .map(|info| format!("; selected adapter was {} ({:?})", info.name, info.backend))
        .unwrap_or_default();
    std::io::Error::other(format!(
        "No hardware-accelerated WGPU adapter is available for {backends:?}{selected}. \
         CPU/WARP/llvmpipe software rendering is disabled."
    ))
}

#[cfg(any(target_os = "windows", test))]
fn cpu_wgpu_adapter_error() -> std::io::Error {
    std::io::Error::other(
        "Software WGPU rendering was requested via SLINT_WGPU_CPU, but Patinae requires \
         a hardware-accelerated WGPU renderer.",
    )
}

fn schedule_pending_save_file_requests(app: Rc<RefCell<App>>, window: slint::Weak<AppWindow>) {
    let should_schedule = {
        let mut app = app.borrow_mut();
        if app.pending_save_file_requests.is_empty() || app.save_file_dialog_scheduled {
            false
        } else {
            app.save_file_dialog_scheduled = true;
            true
        }
    };
    if !should_schedule {
        return;
    }

    let app_for_task = app.clone();
    if let Err(err) = slint::spawn_local(async move {
        process_next_save_file_request(app_for_task, window);
    }) {
        let mut app = app.borrow_mut();
        app.save_file_dialog_scheduled = false;
        log::warn!("Failed to schedule save-file dialog: {err}");
    }
}

fn process_next_save_file_request(app: Rc<RefCell<App>>, window: slint::Weak<AppWindow>) {
    let Some(window) = window.upgrade() else {
        app.borrow_mut().save_file_dialog_scheduled = false;
        return;
    };

    let request = app.borrow_mut().pending_save_file_requests.pop_front();
    let Some(request) = request else {
        app.borrow_mut().save_file_dialog_scheduled = false;
        return;
    };

    #[cfg(target_os = "macos")]
    let selected_path = choose_save_file_path(&window, &request);
    #[cfg(not(target_os = "macos"))]
    let selected_path: Option<String> = None;

    #[cfg(target_os = "macos")]
    let mut needs_redraw = false;
    #[cfg(not(target_os = "macos"))]
    let mut needs_redraw = true;
    let has_more_requests = {
        let mut app = app.borrow_mut();

        #[cfg(not(target_os = "macos"))]
        {
            app.kernel.bus.print_warning(
                "Save dialog is unavailable on this platform; use the ray command with a filename.",
            );
        }

        if let Some(path) = selected_path {
            app.plugins
                .queue_panel_event(save_file_response_event(&request, path));
            needs_redraw = true;
        }

        app.save_file_dialog_scheduled = false;
        !app.pending_save_file_requests.is_empty()
    };

    if needs_redraw {
        window.window().request_redraw();
    }
    if has_more_requests {
        schedule_pending_save_file_requests(app, window.as_weak());
    }
}

#[cfg(target_os = "macos")]
fn choose_save_file_path(app: &AppWindow, request: &SaveFileRequest) -> Option<String> {
    crate::macos::save_file_path(app.window(), request)
}

fn save_file_response_event(request: &SaveFileRequest, path: String) -> PanelEvent {
    PanelEvent {
        panel_id: request.panel_id.clone(),
        control_id: request.reply_control_id.clone(),
        kind: PanelEventKind::TextCommit,
        value: PanelValue::Text(path),
    }
}

fn mix_generation(acc: u64, value: u64) -> u64 {
    acc.rotate_left(13)
        .wrapping_mul(0x9E37_79B1_85EB_CA87)
        .wrapping_add(value)
}

fn viewport_image_signature(image: Option<&ViewportImage>) -> u64 {
    let Some(image) = image else {
        return 0;
    };
    let mut sig = image.width as u64;
    sig = mix_generation(sig, image.height as u64);
    sig = mix_generation(sig, image.data.len() as u64);
    sig = mix_generation(sig, image.data.as_ptr() as usize as u64);
    if let Some(first) = image.data.first() {
        sig = mix_generation(sig, *first as u64);
    }
    if let Some(last) = image.data.last() {
        sig = mix_generation(sig, *last as u64);
    }
    sig
}

fn viewport_image_to_slint(image: Option<&ViewportImage>) -> Option<Image> {
    let image = image?;
    let expected_len = viewport_image_len(image.width, image.height)?;
    if image.width == 0 || image.height == 0 || image.data.len() != expected_len {
        return None;
    }

    let buffer =
        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&image.data, image.width, image.height);
    Some(Image::from_rgba8(buffer))
}

fn viewport_image_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}

fn dispatch_lifecycle_messages(
    messages: &[AppMessage],
    mut quit_event_loop: impl FnMut() -> Result<(), slint::EventLoopError>,
) -> bool {
    if !messages.iter().any(|msg| matches!(msg, AppMessage::Quit)) {
        return false;
    }

    request_event_loop_quit(&mut quit_event_loop);

    true
}

fn request_event_loop_quit(mut quit_event_loop: impl FnMut() -> Result<(), slint::EventLoopError>) {
    if let Err(err) = quit_event_loop() {
        log::warn!("Failed to quit Slint event loop: {err}");
    }
}

fn normalized_slint_shortcut_text(
    key: patinae_scene::KeyCode,
    shift: bool,
    ctrl: bool,
    alt: bool,
    super_key: bool,
) -> Option<&'static str> {
    if alt || !platform_shortcut_modifier_active(ctrl, super_key) {
        return None;
    }

    key.canonical_shortcut_text(shift)
}

#[cfg(target_os = "macos")]
fn platform_shortcut_modifier_active(_ctrl: bool, super_key: bool) -> bool {
    super_key
}

#[cfg(not(target_os = "macos"))]
fn platform_shortcut_modifier_active(ctrl: bool, _super_key: bool) -> bool {
    ctrl
}

fn map_winit_key(
    key: slint::winit_030::winit::keyboard::KeyCode,
) -> Option<patinae_scene::KeyCode> {
    use patinae_scene::KeyCode as Pk;
    use slint::winit_030::winit::keyboard::KeyCode as Wk;

    Some(match key {
        Wk::Digit0 => Pk::Digit0,
        Wk::Digit1 => Pk::Digit1,
        Wk::Digit2 => Pk::Digit2,
        Wk::Digit3 => Pk::Digit3,
        Wk::Digit4 => Pk::Digit4,
        Wk::Digit5 => Pk::Digit5,
        Wk::Digit6 => Pk::Digit6,
        Wk::Digit7 => Pk::Digit7,
        Wk::Digit8 => Pk::Digit8,
        Wk::Digit9 => Pk::Digit9,
        Wk::KeyA => Pk::KeyA,
        Wk::KeyB => Pk::KeyB,
        Wk::KeyC => Pk::KeyC,
        Wk::KeyD => Pk::KeyD,
        Wk::KeyE => Pk::KeyE,
        Wk::KeyF => Pk::KeyF,
        Wk::KeyG => Pk::KeyG,
        Wk::KeyH => Pk::KeyH,
        Wk::KeyI => Pk::KeyI,
        Wk::KeyJ => Pk::KeyJ,
        Wk::KeyK => Pk::KeyK,
        Wk::KeyL => Pk::KeyL,
        Wk::KeyM => Pk::KeyM,
        Wk::KeyN => Pk::KeyN,
        Wk::KeyO => Pk::KeyO,
        Wk::KeyP => Pk::KeyP,
        Wk::KeyQ => Pk::KeyQ,
        Wk::KeyR => Pk::KeyR,
        Wk::KeyS => Pk::KeyS,
        Wk::KeyT => Pk::KeyT,
        Wk::KeyU => Pk::KeyU,
        Wk::KeyV => Pk::KeyV,
        Wk::KeyW => Pk::KeyW,
        Wk::KeyX => Pk::KeyX,
        Wk::KeyY => Pk::KeyY,
        Wk::KeyZ => Pk::KeyZ,
        Wk::F1 => Pk::F1,
        Wk::F2 => Pk::F2,
        Wk::F3 => Pk::F3,
        Wk::F4 => Pk::F4,
        Wk::F5 => Pk::F5,
        Wk::F6 => Pk::F6,
        Wk::F7 => Pk::F7,
        Wk::F8 => Pk::F8,
        Wk::F9 => Pk::F9,
        Wk::F10 => Pk::F10,
        Wk::F11 => Pk::F11,
        Wk::F12 => Pk::F12,
        Wk::Escape => Pk::Escape,
        Wk::Space => Pk::Space,
        Wk::Enter => Pk::Enter,
        Wk::Backspace => Pk::Backspace,
        Wk::Tab => Pk::Tab,
        Wk::Delete => Pk::Delete,
        Wk::ArrowUp => Pk::ArrowUp,
        Wk::ArrowDown => Pk::ArrowDown,
        Wk::ArrowLeft => Pk::ArrowLeft,
        Wk::ArrowRight => Pk::ArrowRight,
        Wk::Home => Pk::Home,
        Wk::End => Pk::End,
        Wk::PageUp => Pk::PageUp,
        Wk::PageDown => Pk::PageDown,
        Wk::Minus => Pk::Minus,
        Wk::Equal => Pk::Equal,
        Wk::BracketLeft => Pk::BracketLeft,
        Wk::BracketRight => Pk::BracketRight,
        Wk::Comma => Pk::Comma,
        Wk::Period => Pk::Period,
        Wk::Slash => Pk::Slash,
        Wk::Backslash => Pk::Backslash,
        Wk::Semicolon => Pk::Semicolon,
        Wk::Quote => Pk::Quote,
        Wk::Backquote => Pk::Backquote,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_render::{
        gib_to_bytes, mib_to_bytes, PERFORMANCE_MAX_BUFFER_SIZE,
        PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE,
    };
    use patinae_scene::KeyCode;

    fn adapter_limits(
        max_buffer_size: u64,
        max_storage_buffer_binding_size: u64,
    ) -> slint::wgpu_29::wgpu::Limits {
        slint::wgpu_29::wgpu::Limits {
            max_buffer_size,
            max_storage_buffer_binding_size,
            max_texture_dimension_1d: 16_384,
            max_texture_dimension_2d: 32_768,
            max_texture_dimension_3d: 2_048,
            ..slint::wgpu_29::wgpu::Limits::default()
        }
    }

    fn temp_startup_path(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("patinae-{name}-{nonce}"))
    }

    fn write_startup_file(name: &str) -> PathBuf {
        let path = temp_startup_path(name);
        std::fs::write(&path, "# startup rc\n").expect("startup rc test file should be writable");
        path
    }

    #[test]
    fn wgpu_required_limits_clamp_to_intel_storage_limit() {
        let adapter_limits = adapter_limits(PERFORMANCE_MAX_BUFFER_SIZE, gib_to_bytes(1) - 4);

        let required =
            patinae_wgpu_required_limits(&adapter_limits, RenderMemoryPolicy::performance());

        assert_eq!(required.max_storage_buffer_binding_size, 1_073_741_820);
        assert!(
            required.max_storage_buffer_binding_size
                <= adapter_limits.max_storage_buffer_binding_size
        );
    }

    #[test]
    fn wgpu_required_limits_preserve_high_end_target() {
        let adapter_limits = adapter_limits(gib_to_bytes(8), u64::MAX);

        let required =
            patinae_wgpu_required_limits(&adapter_limits, RenderMemoryPolicy::performance());

        assert_eq!(required.max_buffer_size, PERFORMANCE_MAX_BUFFER_SIZE);
        assert_eq!(
            required.max_storage_buffer_binding_size,
            PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE
        );
    }

    #[test]
    fn wgpu_required_limits_use_adapter_resolution_limits() {
        let adapter_buffer_size = mib_to_bytes(512);
        let adapter_storage_binding_size = mib_to_bytes(512);
        let adapter_limits = adapter_limits(adapter_buffer_size, adapter_storage_binding_size);

        let required = patinae_wgpu_required_limits(&adapter_limits, RenderMemoryPolicy::lite());

        assert_eq!(required.max_buffer_size, adapter_buffer_size);
        assert_eq!(
            required.max_storage_buffer_binding_size,
            adapter_storage_binding_size
        );
        assert_eq!(required.max_texture_dimension_1d, 16_384);
        assert_eq!(required.max_texture_dimension_2d, 32_768);
        assert_eq!(required.max_texture_dimension_3d, 2_048);
    }

    #[test]
    fn wgpu_windows_default_backend_prefers_dx12() {
        let backends = patinae_wgpu_backends(None, true);

        assert_eq!(backends, slint::wgpu_29::wgpu::Backends::DX12);
    }

    #[test]
    fn wgpu_windows_backend_env_is_respected_without_gl() {
        let backends = patinae_wgpu_backends(
            Some(slint::wgpu_29::wgpu::Backends::VULKAN | slint::wgpu_29::wgpu::Backends::GL),
            true,
        );

        assert_eq!(backends, slint::wgpu_29::wgpu::Backends::VULKAN);
    }

    #[test]
    fn wgpu_non_windows_default_backend_keeps_wgpu_default_without_gl() {
        let backends = patinae_wgpu_backends(None, false);

        assert_eq!(
            backends,
            slint::wgpu_29::wgpu::Backends::default() & !slint::wgpu_29::wgpu::Backends::GL
        );
    }

    #[test]
    fn startup_renderer_error_message_explains_hardware_requirement() {
        let message = startup_renderer_error_message("adapter failed");

        assert!(message.contains("hardware-accelerated WGPU renderer"));
        assert!(message.contains("adapter failed"));
        assert!(message.contains("VM 3D acceleration"));
        assert!(message.contains("Patinae will now exit"));
    }

    #[test]
    fn wgpu_no_hardware_adapter_error_disables_cpu_fallbacks() {
        let err = no_hardware_wgpu_adapter_error(slint::wgpu_29::wgpu::Backends::DX12, None);
        let message = err.to_string();

        assert!(message.contains("No hardware-accelerated WGPU adapter"));
        assert!(message.contains("CPU/WARP/llvmpipe software rendering is disabled"));
    }

    #[test]
    fn wgpu_cpu_override_is_rejected() {
        let message = cpu_wgpu_adapter_error().to_string();

        assert!(message.contains("SLINT_WGPU_CPU"));
        assert!(message.contains("hardware-accelerated WGPU renderer"));
    }

    #[test]
    fn viewport_size_subtracts_logical_panels_at_current_scale() {
        assert_eq!(
            viewport_size_from_parts(1000, 600, 1.0, 200.0, 100.0),
            (752, 500)
        );
        assert_eq!(
            viewport_size_from_parts(1500, 900, 1.5, 200.0, 100.0),
            (1128, 750)
        );
        assert_eq!(
            viewport_size_from_parts(2000, 1200, 2.0, 200.0, 100.0),
            (1504, 1000)
        );
    }

    #[test]
    fn viewport_size_clamps_to_one_physical_pixel() {
        assert_eq!(viewport_size_from_parts(10, 10, 2.0, 200.0, 100.0), (1, 1));
    }

    #[test]
    fn annotation_font_size_stays_device_independent_on_hidpi() {
        let projected = patinae_scene::bridge::ProjectedAnnotationLabel {
            x: 200.0,
            y: 100.0,
            text: "6.1 Å".to_string(),
            color: [0.13, 0.83, 0.93, 1.0],
            size: 14.0,
            alignment: patinae_scene::LabelAlignment::Center,
            owner_id: patinae_scene::RenderObjectId::FIRST,
            insertion_ordinal: 0,
            display_order: 0,
        };

        let label = viewport_label_from_projected(&projected, 2.0);

        assert_eq!(label.x, 100.0);
        assert_eq!(label.y, 50.0);
        assert_eq!(label.size, 14.0);
    }

    #[test]
    fn lifecycle_dispatch_quits_event_loop_once() {
        let messages = [
            AppMessage::TogglePanel("objects".into()),
            AppMessage::Quit,
            AppMessage::Quit,
        ];
        let mut calls = 0;

        let quit_requested = dispatch_lifecycle_messages(&messages, || {
            calls += 1;
            Ok(())
        });

        assert!(quit_requested);
        assert_eq!(calls, 1);
    }

    #[test]
    fn lifecycle_dispatch_ignores_non_quit_messages() {
        let messages = [AppMessage::TogglePanel("objects".into())];
        let mut calls = 0;

        let quit_requested = dispatch_lifecycle_messages(&messages, || {
            calls += 1;
            Ok(())
        });

        assert!(!quit_requested);
        assert_eq!(calls, 0);
    }

    #[test]
    fn request_event_loop_quit_calls_delegate_once() {
        let mut calls = 0;

        request_event_loop_quit(|| {
            calls += 1;
            Ok(())
        });

        assert_eq!(calls, 1);
    }

    #[test]
    fn save_file_response_targets_originating_panel() {
        let request = SaveFileRequest {
            panel_id: "rt_toolbar".into(),
            reply_control_id: "save_file_selected".into(),
            title: "Save ray-traced image".into(),
            default_file_name: "raytrace.png".into(),
            allowed_extensions: vec!["png".into()],
        };

        let event = save_file_response_event(&request, "/tmp/render.png".into());

        assert_eq!(event.panel_id, "rt_toolbar");
        assert_eq!(event.control_id, "save_file_selected");
        assert_eq!(event.kind, PanelEventKind::TextCommit);
        assert!(matches!(event.value, PanelValue::Text(path) if path == "/tmp/render.png"));
    }

    #[test]
    fn save_file_requests_are_deferred_out_of_render_path() {
        let request = SaveFileRequest {
            panel_id: "rt_toolbar".into(),
            reply_control_id: "save_file_selected".into(),
            title: "Save ray-traced image".into(),
            default_file_name: "raytrace.png".into(),
            allowed_extensions: vec!["png".into()],
        };
        let mut bus = patinae_framework::message::MessageBus::new();
        topics::publish(&mut bus, SAVE_FILE_REQUEST_TOPIC, &request);
        let messages = bus.drain_outbox();

        let mut app = App::new();

        assert!(app.queue_save_file_request(&messages[0]));
        assert_eq!(app.pending_save_file_requests.pop_front(), Some(request));
        assert!(!app.save_file_dialog_scheduled);
    }

    #[test]
    fn startup_actions_skip_missing_default_patinaerc() {
        let path = temp_startup_path("missing-default-patinaerc");
        let actions = startup_actions(
            PatinaercPath {
                path,
                source: PatinaercSource::Default,
            },
            Vec::new(),
        );

        assert!(actions.is_empty());
    }

    #[test]
    fn startup_actions_warn_for_missing_explicit_patinaerc() {
        let path = temp_startup_path("missing-explicit-patinaerc");
        let actions: Vec<_> = startup_actions(
            PatinaercPath {
                path: path.clone(),
                source: PatinaercSource::Env,
            },
            Vec::new(),
        )
        .into_iter()
        .collect();

        assert!(matches!(
            actions.as_slice(),
            [StartupAction::Warning(message)]
                if message.contains("PATINAERC") && message.contains(&path.display().to_string())
        ));
    }

    #[test]
    fn startup_actions_run_patinaerc_before_argv_load() {
        let path = write_startup_file("existing-patinaerc");
        let actions: Vec<_> = startup_actions(
            PatinaercPath {
                path: path.clone(),
                source: PatinaercSource::Default,
            },
            vec![PathBuf::from("input.pdb")],
        )
        .into_iter()
        .collect();
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            actions,
            vec![
                StartupAction::RunPatinaerc(path),
                StartupAction::RouteArgvFile(PathBuf::from("input.pdb"))
            ]
        );
    }

    #[test]
    fn startup_actions_keep_multiple_argv_files_in_order() {
        let actions: Vec<_> = startup_actions(
            PatinaercPath {
                path: temp_startup_path("missing-default-patinaerc"),
                source: PatinaercSource::Default,
            },
            vec![PathBuf::from("first.pdb"), PathBuf::from("second.pml")],
        )
        .into_iter()
        .collect();

        assert_eq!(
            actions,
            vec![
                StartupAction::RouteArgvFile(PathBuf::from("first.pdb")),
                StartupAction::RouteArgvFile(PathBuf::from("second.pml")),
            ]
        );
    }

    #[test]
    fn startup_command_paths_with_spaces_are_quoted() {
        assert_eq!(
            quote_command_arg("/tmp/patinae startup/patinaerc"),
            "\"/tmp/patinae startup/patinaerc\""
        );
    }

    #[test]
    fn recent_open_command_paths_with_spaces_are_quoted() {
        assert_eq!(
            recent_open_command_for_existing_path(Path::new("/tmp/recent files/1fsd.pdb")),
            "load \"/tmp/recent files/1fsd.pdb\""
        );
    }

    #[test]
    fn standard_panel_preset_only_applies_when_empty_scene_becomes_active() {
        assert!(should_apply_standard_panel_preset(true, false));
        assert!(!should_apply_standard_panel_preset(true, true));
        assert!(!should_apply_standard_panel_preset(false, false));
        assert!(!should_apply_standard_panel_preset(false, true));
    }

    #[test]
    fn repl_pill_hit_test_blocks_only_the_floating_pill() {
        assert!(App::point_over_repl_pill(
            (400.0, 460.0),
            800,
            500,
            1.0,
            true
        ));
        assert!(!App::point_over_repl_pill(
            (400.0, 430.0),
            800,
            500,
            1.0,
            true
        ));
        assert!(!App::point_over_repl_pill(
            (400.0, 460.0),
            800,
            500,
            1.0,
            false
        ));

        assert!(App::point_over_repl_pill(
            (400.0, 460.0),
            1600,
            1000,
            2.0,
            true
        ));
    }

    fn platform_modifiers() -> (bool, bool) {
        #[cfg(target_os = "macos")]
        {
            (false, true)
        }
        #[cfg(not(target_os = "macos"))]
        {
            (true, false)
        }
    }

    #[test]
    fn normalizes_common_platform_shortcuts() {
        let (ctrl, super_key) = platform_modifiers();

        for (key, text) in [
            (KeyCode::KeyA, "a"),
            (KeyCode::KeyC, "c"),
            (KeyCode::KeyL, "l"),
            (KeyCode::KeyV, "v"),
            (KeyCode::KeyX, "x"),
            (KeyCode::KeyZ, "z"),
            (KeyCode::KeyU, "u"),
        ] {
            assert_eq!(
                normalized_slint_shortcut_text(key, false, ctrl, false, super_key),
                Some(text)
            );
        }
    }

    #[test]
    fn normalizes_shifted_printable_shortcuts() {
        let (ctrl, super_key) = platform_modifiers();

        assert_eq!(
            normalized_slint_shortcut_text(KeyCode::KeyZ, true, ctrl, false, super_key),
            Some("Z")
        );
        assert_eq!(
            normalized_slint_shortcut_text(KeyCode::Digit1, true, ctrl, false, super_key),
            Some("!")
        );
        assert_eq!(
            normalized_slint_shortcut_text(KeyCode::Slash, true, ctrl, false, super_key),
            Some("?")
        );
    }

    #[test]
    fn skips_shortcuts_without_platform_modifier() {
        assert_eq!(
            normalized_slint_shortcut_text(KeyCode::KeyA, false, false, false, false),
            None
        );

        #[cfg(target_os = "macos")]
        assert_eq!(
            normalized_slint_shortcut_text(KeyCode::KeyA, false, true, false, false),
            None
        );

        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            normalized_slint_shortcut_text(KeyCode::KeyA, false, false, false, true),
            None
        );
    }

    #[test]
    fn skips_alt_and_non_printable_shortcuts() {
        let (ctrl, super_key) = platform_modifiers();

        assert_eq!(
            normalized_slint_shortcut_text(KeyCode::KeyA, false, ctrl, true, super_key),
            None
        );
        assert_eq!(
            normalized_slint_shortcut_text(KeyCode::Escape, false, ctrl, false, super_key),
            None
        );
        assert_eq!(
            normalized_slint_shortcut_text(KeyCode::F1, false, ctrl, false, super_key),
            None
        );
    }
}
