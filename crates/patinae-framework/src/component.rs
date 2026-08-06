//! Component System
//!
//! Defines the [`Component`] trait for self-contained UI panels and
//! [`SharedContext`] for read-only access to application state.

use patinae_cmd::{CommandRegistry, DynamicSettingRegistry, ResolvedSetting};
use patinae_color::NamedPalette;
use patinae_scene::{Camera, Movie, ObjectRegistry, RecentAtoms, SelectionManager, ViewportImage};
use patinae_settings::{SettingValue, Settings};

use crate::atom_stream::{AtomStreamPlan, AtomStreamRequest, AtomStreamRows};
use crate::message::AppMessage;

/// Read-only snapshot of application state, shared with all components.
///
/// Components read this context during rendering but never mutate it directly.
/// All mutations go through [`crate::message::MessageBus`].
pub struct SharedContext<'a> {
    // Scene state
    pub registry: &'a ObjectRegistry,
    pub camera: &'a Camera,
    pub selections: &'a SelectionManager,
    pub recent_atoms: &'a RecentAtoms,
    pub named_palette: &'a NamedPalette,
    pub movie: &'a Movie,
    pub settings: &'a Settings,
    pub clear_color: [f32; 3],

    // GPU (optional — available when a wgpu surface is initialised)
    pub gpu_device: Option<&'a wgpu::Device>,
    pub gpu_queue: Option<&'a wgpu::Queue>,

    // Change detection
    /// Monotonically increasing counter bumped on every scene change
    /// (camera, objects, commands, etc.). Compare against a cached value
    /// to detect viewport changes.
    pub scene_generation: u64,

    // Viewport image overlay (e.g. from `ray` command or plugins)
    pub viewport_image: Option<&'a ViewportImage>,

    // Command system (for autocomplete)
    pub command_names: &'a [String],
    pub command_registry: &'a CommandRegistry,
    pub setting_names: &'a [&'a str],
    /// Dynamic settings registry for advanced host/runtime plumbing.
    ///
    /// Components should use [`Self::resolve_setting`], [`Self::setting_value`],
    /// or the typed `setting_*` helpers for ordinary setting reads.
    pub dynamic_settings: Option<&'a DynamicSettingRegistry>,
}

impl<'a> SharedContext<'a> {
    /// Resolve a built-in or dynamic setting by name.
    ///
    /// Built-in settings take precedence over dynamic plugin settings.
    #[must_use]
    pub fn resolve_setting(&self, name: &str) -> Option<ResolvedSetting> {
        ResolvedSetting::lookup(name, self.dynamic_settings)
    }

    /// Read a setting value by name.
    ///
    /// This reads built-in settings first, then dynamic plugin settings.
    /// Dynamic settings fall back to their descriptor default after `unset`.
    #[must_use]
    pub fn setting_value(&self, name: &str) -> Option<SettingValue> {
        self.resolve_setting(name)
            .map(|setting| setting.global_value_from_settings(self.settings))
    }

    /// Read a setting as a boolean.
    ///
    /// Returns `default` when the setting is not registered, or when neither
    /// the current value nor the descriptor default can be converted to a
    /// boolean.
    #[must_use]
    pub fn setting_bool(&self, name: &str, default: bool) -> bool {
        self.setting_value(name)
            .and_then(|value| value.as_bool())
            .unwrap_or(default)
    }

    /// Read a setting as an integer.
    ///
    /// Returns `default` when the setting is not registered, or when neither
    /// the current value nor the descriptor default can be converted to an
    /// integer.
    #[must_use]
    pub fn setting_int(&self, name: &str, default: i32) -> i32 {
        self.setting_value(name)
            .and_then(|value| value.as_int())
            .unwrap_or(default)
    }

    /// Read a setting as a float.
    ///
    /// Returns `default` when the setting is not registered, or when neither
    /// the current value nor the descriptor default can be converted to a
    /// float.
    #[must_use]
    pub fn setting_float(&self, name: &str, default: f32) -> f32 {
        self.setting_value(name)
            .and_then(|value| value.as_float())
            .unwrap_or(default)
    }

    /// Opens a borrowed atom iterator.
    ///
    /// # Errors
    /// Returns an error when the request references missing objects or invalid
    /// selection expressions.
    pub fn iter_atoms(&self, request: &AtomStreamRequest) -> Result<AtomStreamRows<'a>, String> {
        let plan = AtomStreamPlan::open(self, request)?;
        Ok(plan.iter(self.registry))
    }

    /// Opens a reusable atom stream plan.
    ///
    /// # Errors
    /// Returns an error when the request cannot be resolved against this scene.
    pub fn atom_stream_plan(&self, request: &AtomStreamRequest) -> Result<AtomStreamPlan, String> {
        AtomStreamPlan::open(self, request)
    }
}

/// A self-contained UI component (panel).
///
/// Each component owns its domain model and UI state.
/// It reads shared application state via [`SharedContext`] and writes
/// actions via [`crate::message::MessageBus`].
///
/// # Design
///
/// - `on_message()` lets components react to dispatched messages
/// - The 3D viewport is intentionally **not** a Component
pub trait Component: std::any::Any {
    /// Unique string identifier (e.g., `"command_line"`, `"sequence"`).
    fn id(&self) -> &'static str;

    /// Display name for panel headers and tabs.
    fn title(&self) -> &str;

    /// React to a dispatched message. Called once per message after dispatch.
    ///
    /// Default implementation: no-op.
    fn on_message(&mut self, _msg: &AppMessage) {}

    // =========================================================================
    // Lifecycle hooks (default no-ops)
    // =========================================================================

    /// Called once after the component is registered.
    fn on_init(&mut self) {}

    /// Called when the component's panel gains focus (e.g. tab selected).
    fn on_focus(&mut self) {}

    /// Called when the component's panel loses focus (e.g. tab deselected).
    fn on_blur(&mut self) {}
}
