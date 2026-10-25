//! Native host configuration and diagnostics for renderer startup decisions.

use patinae_render::{
    select_render_memory_policy, NativeGpuMemoryBudget, RenderMemoryPolicy, RenderMemoryProfile,
    RenderMemorySelectionInput,
};

/// Applies the host override and logs the selector's evidence without reinterpreting it.
pub(crate) fn memory_policy(
    adapter: &wgpu::AdapterInfo,
    limits: &wgpu::Limits,
    budget: Option<NativeGpuMemoryBudget>,
    phase: &str,
) -> RenderMemoryPolicy {
    const ENV_PROFILE: &str = "PATINAE_RENDER_MEMORY_PROFILE";
    let override_profile = std::env::var(ENV_PROFILE).ok().and_then(|value| {
        match value.parse::<RenderMemoryProfile>() {
            Ok(profile) => Some(profile),
            Err(error) => {
                log::warn!("ignoring invalid {ENV_PROFILE}={value:?}: {error}");
                None
            }
        }
    });
    let selection = select_render_memory_policy(
        RenderMemorySelectionInput::from_wgpu(adapter, limits, false),
        budget,
        override_profile,
    );
    log::info!(
        "render memory selection: phase={phase} adapter={} backend={:?} profile={} reason={:?} max_buffer_size={} max_storage_buffer_binding_size={} native_headroom={:?}",
        adapter.name, adapter.backend, selection.policy.profile, selection.reason,
        limits.max_buffer_size, limits.max_storage_buffer_binding_size, selection.headroom,
    );
    selection.policy
}
