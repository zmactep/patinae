//! Preparation for ViewportRenderer.

use super::*;

impl ViewportRenderer {
    pub(super) fn from_prepared(
        state: RenderState,
        config: RenderConfig,
        wgpu_errors: WgpuErrorFlags,
    ) -> Self {
        let memory_policy = config.memory;
        let recovery = RenderMemoryRecoveryStage::normal(memory_policy.profile);
        Self {
            state,
            config,
            auto_memory_policy: memory_policy,
            recovery,
            wgpu_errors,
            last_viewport_size: None,
            render_scene: CachedRenderScene::default(),
            projected_labels: ProjectedSceneLabels::default(),
            last_registry_generation: 0,
            pending_hover: None,
            last_hover_submit: None,
            color_textures: Vec::new(),
            color_textures_next: 0,
            viewport_gpu_image: None,
            memory_policy,
            warned_unsupported_memory_policy: false,
        }
    }

    /// Capture owned GPU handles; defer all preparation until after the first frame.
    pub(crate) fn schedule(
        api: &slint::GraphicsAPI,
    ) -> Option<crate::renderer_startup::RendererStartup<PreparedViewport>> {
        let slint::GraphicsAPI::WGPU30 { device, queue, .. } = api else {
            return None;
        };
        let (device, queue) = (device.clone(), queue.clone());
        Some(crate::renderer_startup::RendererStartup::new(move || {
            Self::prepare(device, queue)
        }))
    }

    pub(super) fn prepare(device: wgpu::Device, queue: wgpu::Queue) -> PreparedViewport {
        // Explicit diagnostic fault injection, bounded to avoid accidental long stalls.
        if let Some(delay) = std::env::var("PATINAE_DEBUG_RENDER_PREPARE_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|delay| *delay > 0)
        {
            let delay = delay.min(30_000);
            log::info!("Viewport diagnostic preparation delay: {delay} ms");
            std::thread::sleep(Duration::from_millis(delay));
        }
        let started = Instant::now();
        let adapter_info = device.adapter_info();
        let info = Some(AdapterInfo {
            device_name: adapter_info.name.clone(),
            backend: format!("{:?}", adapter_info.backend),
        });
        let memory_policy = render_memory_policy_for_device(
            &adapter_info,
            &device.limits(),
            crate::components::gpu_memory::device_memory_budget(&device),
            "PATINAE_RENDER_MEMORY_PROFILE",
        );
        log::info!(
            "render memory profile: profile={} budget={:?} handoff_ring={}",
            memory_policy.profile,
            memory_policy.budget_bytes,
            memory_policy.frame_targets.viewport_handoff_ring
        );
        log_wgpu_device_diagnostics(&device);
        let wgpu_errors = install_wgpu_error_handlers(&device);

        let config = RenderConfig {
            memory: memory_policy,
            ..Default::default()
        };
        let state = RenderState::prepare_with_config(
            Arc::new(device),
            Arc::new(queue),
            VIEWPORT_FORMAT,
            (1, 1),
            config,
        );
        log::info!(
            "Viewport preparation: {:.3} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        PreparedViewport {
            state,
            config,
            wgpu_errors,
            info,
        }
    }
}
pub(super) fn log_wgpu_device_diagnostics(device: &wgpu::Device) {
    let limits = device.limits();
    let features = device.features();
    log::info!(
        "wgpu device limits: max_storage_buffer_binding_size={} max_buffer_size={} \
         max_compute_workgroups_per_dimension={} max_texture_dimension_2d={}",
        limits.max_storage_buffer_binding_size,
        limits.max_buffer_size,
        limits.max_compute_workgroups_per_dimension,
        limits.max_texture_dimension_2d,
    );
    log::info!(
        "wgpu device features: buffer_binding_array={} storage_resource_binding_array={} timestamp_query={}",
        features.contains(wgpu::Features::BUFFER_BINDING_ARRAY),
        features.contains(wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY),
        features.contains(wgpu::Features::TIMESTAMP_QUERY),
    );
}

pub(super) fn install_wgpu_error_handlers(device: &wgpu::Device) -> WgpuErrorFlags {
    let flags = WgpuErrorFlags::default();
    let uncaptured = flags.clone();
    device.on_uncaptured_error(Arc::new(move |error| {
        if is_wgpu_oom(&error) {
            uncaptured.mark_oom();
            log::warn!("Uncaptured WGPU OOM: {error}");
        } else {
            log::error!("Uncaptured WGPU error: {error}");
        }
    }));
    let lost = flags.clone();
    device.set_device_lost_callback(move |reason, message| {
        lost.mark_device_lost();
        log::error!("WGPU device lost: {reason:?}: {message}");
    });
    flags
}

pub(crate) fn render_memory_policy_for_device(
    adapter_info: &wgpu::AdapterInfo,
    limits: &wgpu::Limits,
    budget: Option<patinae_render::NativeGpuMemoryBudget>,
    env_var: &str,
) -> RenderMemoryPolicy {
    let override_profile = std::env::var(env_var).ok().and_then(|value| match value
        .parse::<RenderMemoryProfile>(
    ) {
        Ok(profile) => Some(profile),
        Err(err) => {
            log::warn!("ignoring invalid {env_var}={value:?}: {err}");
            None
        }
    });
    match budget {
        Some(budget) => log::info!(
            "native GPU memory: source={:?} budget={:.2} MiB process_usage={:.2} MiB headroom={:.2} MiB",
            budget.source,
            patinae_render::bytes_to_mib(budget.budget_bytes),
            patinae_render::bytes_to_mib(budget.usage_bytes),
            patinae_render::bytes_to_mib(budget.headroom_bytes()),
        ),
        None => log::info!("native GPU memory budget unavailable; using capability fallback"),
    }
    select_render_memory_policy_with_budget(
        RenderMemorySelectionInput::from_wgpu(adapter_info, limits, false),
        budget,
        override_profile,
    )
}

pub(super) fn memory_policy_supported_by_device(
    policy: RenderMemoryPolicy,
    limits: &wgpu::Limits,
) -> bool {
    match policy.profile {
        RenderMemoryProfile::Performance => {
            limits.max_buffer_size >= PERFORMANCE_MIN_BUFFER_SIZE
                && limits.max_storage_buffer_binding_size
                    >= PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE
        }
        RenderMemoryProfile::Balanced
        | RenderMemoryProfile::Lite
        | RenderMemoryProfile::Manual { .. } => true,
    }
}
