//! Native application ownership and subsystem orchestration.

pub(crate) mod bridges;
pub(crate) mod native_menu;

mod effects;
pub(crate) mod events;
mod input_events;
mod window_events;

mod gpu_config;
use gpu_config::*;
mod bootstrap;
#[cfg(any(target_os = "windows", test))]
use bootstrap::cpu_wgpu_adapter_error;
use bootstrap::no_hardware_wgpu_adapter_error;
pub(crate) use bootstrap::run;
#[cfg(test)]
use bootstrap::startup_renderer_error_message;
mod interaction;
use interaction::*;
mod diagnostics;
use diagnostics::*;
mod startup;
use startup::*;
mod notifications;

mod plugins;
use plugins::*;
mod frame;
use frame::*;
mod host;
use host::*;
mod files;
#[cfg(test)]
use files::*;
mod rendering;

use patinae_scene::manipulation::{ViewportCursor, ViewportManipulation, ViewportTargetId};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, Model, Rgba8Pixel, SharedPixelBuffer};

use patinae_cmd::AnnotationRequest;
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
    bridge::ProjectedAnnotationLabel, canonical_atom_path_for_hit, expand_pick_to_selection,
    pick_expression_for_hit, CaptureRenderer, KeyBinding, PickHit, ViewportImage,
};
use patinae_select::build_sele_command;
use patinae_settings::{
    paths::{PatinaercPath, PatinaercSource},
    MouseSelectionMode,
};

use crate::bridges::input::{InputBridge, PendingClick};
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
use crate::display_recovery::{log_window_event, request_display_recovery_redraw, DisplayRecovery};
use crate::native_file_actions::{enqueue_file_action, quote_command_arg, NativeFileSource};
use crate::plugin_startup::PluginStartup;
use crate::recent_files::RecentFilesBridge;
use crate::recent_thumbnails::{RECENT_THUMBNAIL_HEIGHT, RECENT_THUMBNAIL_WIDTH};
use crate::startup_alert;
use crate::{
    AppWindow, LayoutState, MenuState, NotificationState, Theme, ViewportLabel, ViewportState,
};

const TRANSIENT_NOTIFICATION_SECS: u64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupAction {
    Warning(String),
    RunPatinaerc(PathBuf),
    RouteArgvFile(PathBuf),
}

// ---------------------------------------------------------------------------
// App — thin orchestrator wiring kernel + bridges + renderer
// ---------------------------------------------------------------------------

struct App {
    effects: Vec<effects::Effect>,
    events: events::EventSender,
    interaction: interaction::Interaction,
    rendering: rendering::Rendering,
    plugin_runtime: plugins::Plugins,
    diagnostics: diagnostics::Diagnostics,
    file_actions: files::FileActions,
    kernel: AppKernel,
    objects: ObjectsBridge,
    movie: MovieBridge,
    repl: ReplBridge,
    sequence: SequenceBridge,
    viewport: ViewportBridge,
    platform: PlatformBridge,
    theme: bridges::theme::ThemeBridge,
    last_animation_instant: Cell<Option<Instant>>,
    startup_actions: VecDeque<StartupAction>,
    transient_notification: Option<(String, Instant)>,
    host_timer: slint::Timer,
    last_empty_mode: bool,
}

impl App {
    fn new() -> Self {
        let perf_enabled = std::env::var("PATINAE_PERF")
            .ok()
            .map(|v| v != "0" && !v.is_empty())
            .unwrap_or(false);
        let mut kernel = AppKernel::new();
        kernel.set_async_command_handler(Some(Box::new(crate::fetch::task_from_request)));
        let plugins = PluginHost::new();
        let command_names_cache = kernel
            .executor
            .registry()
            .names()
            .map(|s| s.to_string())
            .collect();
        let setting_names_cache = patinae_settings::setting_names();
        Self {
            effects: Vec::new(),
            events: events::EventSender::default(),
            kernel,
            objects: ObjectsBridge::new(),
            movie: MovieBridge::new(),
            repl: ReplBridge::new(),
            sequence: SequenceBridge::new(),
            viewport: ViewportBridge::new(800, 500),
            platform: PlatformBridge::new(),
            theme: bridges::theme::ThemeBridge::default(),
            last_animation_instant: Cell::new(None),
            startup_actions: VecDeque::new(),
            transient_notification: None,
            host_timer: slint::Timer::default(),
            last_empty_mode: true,
            interaction: interaction::Interaction {
                input: InputBridge::new(),
                manipulation: ViewportManipulation::default(),
                last_hover_mode: None,
                winit_modifiers: Cell::new((false, false, false, false)),
                cursor_pos: Cell::new((0.0, 0.0)),
                cursor_known: Cell::new(false),
            },
            rendering: rendering::Rendering {
                renderer: None,
                renderer_startup: None,
                display_recovery: Cell::new(DisplayRecovery::default()),
            },
            plugin_runtime: plugins::Plugins {
                plugins,
                plugin_bridge: PluginBridge::new(),
                plugin_startup: PluginStartup::new(Instant::now()),
                plugin_list: Default::default(),
                command_names_cache,
                setting_names_cache,
                last_plugin_sync_key: None,
            },
            diagnostics: diagnostics::Diagnostics {
                perf_enabled,
                last_frame_instant: Cell::new(None),
                perf_history: RefCell::new(FrameStatsHistory::with_default_capacity()),
                perf_update_counter: Cell::new(0),
            },
            file_actions: files::FileActions {
                recent_files: RecentFilesBridge::new(
                    patinae_settings::paths::recent_files_path(),
                    patinae_settings::paths::thumbnail_cache_dir().join("recent"),
                ),
                pending_recent_thumbnail: None,
                deferred_file_actions: VecDeque::new(),
                pending_save_file_requests: VecDeque::new(),
                pending_annotation_requests: VecDeque::new(),
                save_file_dialog_scheduled: false,
            },
        }
    }

    // --- Renderer lifecycle ---

    // --- Per-frame rendering ---

    // --- Picking ---

    // --- Helpers ---
}

// ---------------------------------------------------------------------------
// run() — entry point
// ---------------------------------------------------------------------------

const STARTUP_RENDERER_ALERT_TITLE: &str = "Patinae Renderer Unavailable";

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../ui_tests.rs"]
mod ui_tests;
