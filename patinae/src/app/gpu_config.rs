//! Gpu config for the native application.

use super::*;

#[cfg(any(not(target_os = "windows"), test))]
pub(super) fn patinae_wgpu_required_limits(
    adapter_limits: &slint::wgpu_30::wgpu::Limits,
    memory_policy: RenderMemoryPolicy,
) -> slint::wgpu_30::wgpu::Limits {
    required_limits_for_memory_policy(adapter_limits, memory_policy)
}

pub(super) fn patinae_wgpu_backends(
    env_backends: Option<slint::wgpu_30::wgpu::Backends>,
    is_windows: bool,
) -> slint::wgpu_30::wgpu::Backends {
    let backends = if is_windows {
        env_backends.unwrap_or(slint::wgpu_30::wgpu::Backends::DX12)
    } else {
        env_backends.unwrap_or_default()
    };
    backends & !slint::wgpu_30::wgpu::Backends::GL
}

pub(super) fn patinae_wgpu_settings() -> slint::wgpu_30::WGPUSettings {
    let mut settings = slint::wgpu_30::WGPUSettings::default();
    settings.power_preference = slint::wgpu_30::wgpu::PowerPreference::HighPerformance;
    settings.backends = patinae_wgpu_backends(
        slint::wgpu_30::wgpu::Backends::from_env(),
        cfg!(target_os = "windows"),
    );
    settings.device_required_limits = slint::wgpu_30::wgpu::Limits::default();
    settings
        .device_required_limits
        .max_storage_buffers_per_shader_stage = RENDER_COMPUTE_STORAGE_BUFFERS_PER_STAGE;
    settings
}

pub(super) fn patinae_wgpu_instance(
    settings: &slint::wgpu_30::WGPUSettings,
) -> slint::wgpu_30::wgpu::Instance {
    pollster::block_on(
        slint::wgpu_30::wgpu::util::new_instance_with_webgpu_detection(
            slint::wgpu_30::wgpu::InstanceDescriptor {
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
pub(super) fn patinae_wgpu_configuration(
) -> Result<slint::wgpu_30::WGPUConfiguration, Box<dyn std::error::Error>> {
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
    Ok(slint::wgpu_30::WGPUConfiguration::Automatic(settings))
}

#[cfg(not(target_os = "windows"))]
pub(super) fn patinae_wgpu_configuration(
) -> Result<slint::wgpu_30::WGPUConfiguration, Box<dyn std::error::Error>> {
    let mut settings = patinae_wgpu_settings();

    let instance = patinae_wgpu_instance(&settings);

    let adapter = pollster::block_on(async {
        match slint::wgpu_30::wgpu::util::initialize_adapter_from_env(&instance, None).await {
            Ok(adapter) => Ok(adapter),
            Err(_) => {
                instance
                    .request_adapter(&slint::wgpu_30::wgpu::RequestAdapterOptions {
                        power_preference: settings.power_preference,
                        force_fallback_adapter: false,
                        compatible_surface: None,
                        ..Default::default()
                    })
                    .await
            }
        }
    })?;

    let adapter_limits = adapter.limits();
    let adapter_features = adapter.features();
    let adapter_info = adapter.get_info();
    if adapter_info.device_type == slint::wgpu_30::wgpu::DeviceType::Cpu {
        return Err(no_hardware_wgpu_adapter_error(settings.backends, Some(&adapter_info)).into());
    }
    log_selected_gpu(&adapter_info);
    let memory_policy = render_memory_policy_for_device(
        &adapter_info,
        &adapter_limits,
        crate::components::gpu_memory::adapter_memory_budget(&adapter),
        "PATINAE_RENDER_MEMORY_PROFILE",
    );
    crate::allocator_startup::apply(memory_policy.profile)?;
    if let Some(notice) = crate::allocator_startup::restart_notice(memory_policy.profile) {
        log::info!("{notice}");
    }
    settings.device_memory_hints = memory_policy.wgpu_memory_hints();
    let binding_array_features = slint::wgpu_30::wgpu::Features::BUFFER_BINDING_ARRAY
        | slint::wgpu_30::wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY;
    let requested_binding_array_features = adapter_features & binding_array_features;
    let requested_timestamp_query_feature =
        adapter_features & slint::wgpu_30::wgpu::Features::TIMESTAMP_QUERY;
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
            .contains(slint::wgpu_30::wgpu::Features::BUFFER_BINDING_ARRAY),
        requested_binding_array_features
            .contains(slint::wgpu_30::wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY),
        requested_timestamp_query_feature.contains(slint::wgpu_30::wgpu::Features::TIMESTAMP_QUERY),
    );
    log::info!(
        "render memory profile: profile={} budget={:?} memory_hints={:?}",
        memory_policy.profile,
        memory_policy.budget_bytes,
        settings.device_memory_hints
    );

    let (device, queue) = pollster::block_on(adapter.request_device(
        &slint::wgpu_30::wgpu::DeviceDescriptor {
            label: settings.device_label.as_deref(),
            required_features: settings.device_required_features,
            required_limits,
            experimental_features: settings.device_experimental_features,
            memory_hints: settings.device_memory_hints,
            trace: slint::wgpu_30::wgpu::Trace::default(),
        },
    ))?;

    Ok(slint::wgpu_30::WGPUConfiguration::Manual {
        instance,
        adapter,
        device,
        queue,
    })
}

#[cfg(target_os = "windows")]
pub(super) fn log_windows_wgpu_limits(prefix: &str, settings: &slint::wgpu_30::WGPUSettings) {
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
pub(super) fn patinae_windows_has_hardware_adapter(
    settings: &slint::wgpu_30::WGPUSettings,
) -> bool {
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
        has_hardware |= info.device_type != slint::wgpu_30::wgpu::DeviceType::Cpu;
    }
    has_hardware
}
