//! Gpu config for the native application.

use super::*;

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
    if std::env::var_os("SLINT_WGPU_CPU").is_some() {
        return Err(cpu_wgpu_adapter_error().into());
    }

    settings.device_required_limits = patinae_windows_required_limits(&settings)?;
    log_windows_wgpu_limits("wgpu Windows configuration", &settings);

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
    let memory_policy = crate::gpu_startup::memory_policy(
        &adapter_info,
        &adapter_limits,
        patinae_render::adapter_memory_budget(&adapter),
        "provisional-adapter",
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
    let required_limits = required_device_limits(&adapter_limits)?;
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
        "render memory profile: profile={} allocation_budget={:?} memory_hints={:?}",
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
fn patinae_windows_required_limits(
    settings: &slint::wgpu_30::WGPUSettings,
) -> Result<slint::wgpu_30::wgpu::Limits, Box<dyn std::error::Error>> {
    let instance = patinae_wgpu_instance(settings);
    let adapters = pollster::block_on(instance.enumerate_adapters(settings.backends));
    let mut candidates = Vec::with_capacity(adapters.len());
    for adapter in &adapters {
        let info = adapter.get_info();
        let limits = adapter.limits();
        log::info!(
            "wgpu Windows adapter preflight: name={} backend={:?} type={:?} max_buffer_size={} max_storage_buffer_binding_size={} max_storage_buffers_per_shader_stage={}",
            info.name,
            info.backend,
            info.device_type,
            limits.max_buffer_size,
            limits.max_storage_buffer_binding_size,
            limits.max_storage_buffers_per_shader_stage,
        );
        candidates.push((info, limits));
    }
    let desired_name = std::env::var("WGPU_ADAPTER_NAME").ok();
    let limits = windows_candidate_limits(&candidates, desired_name.as_deref())?;
    for (info, candidate) in &candidates {
        if info.device_type != slint::wgpu_30::wgpu::DeviceType::Cpu
            && desired_name
                .as_ref()
                .is_none_or(|name| info.name.to_lowercase().contains(&name.to_lowercase()))
            && (candidate.max_buffer_size > limits.max_buffer_size
                || candidate.max_storage_buffer_binding_size
                    > limits.max_storage_buffer_binding_size)
        {
            log::info!("wgpu Windows candidate {} has higher limits than the shared request; Patinae targets and the weakest matching hardware candidate cap the request", info.name);
        }
    }
    log::info!("wgpu Windows shared candidate limits: max_buffer_size={} max_storage_buffer_binding_size={}; automatic surface-compatible adapter selection retained", limits.max_buffer_size, limits.max_storage_buffer_binding_size);
    Ok(limits)
}

/// Negotiate limits before Slint selects a surface-compatible hardware adapter.
#[cfg(any(target_os = "windows", test))]
pub(super) fn windows_candidate_limits(
    candidates: &[(
        slint::wgpu_30::wgpu::AdapterInfo,
        slint::wgpu_30::wgpu::Limits,
    )],
    desired_name: Option<&str>,
) -> Result<slint::wgpu_30::wgpu::Limits, String> {
    use slint::wgpu_30::wgpu::{DeviceType, Limits};
    let desired_name = desired_name.map(str::to_lowercase);
    let mut common: Option<Limits> = None;
    for (info, limits) in candidates {
        if desired_name
            .as_ref()
            .is_some_and(|name| !info.name.to_lowercase().contains(name))
        {
            continue;
        }
        if info.device_type == DeviceType::Cpu {
            // Slint's name override can select a CPU before a similarly named GPU.
            if desired_name.is_some() {
                return Err(format!(
                    "WGPU_ADAPTER_NAME matches CPU adapter {}; select a hardware GPU",
                    info.name
                ));
            }
            continue;
        }
        let candidate = required_device_limits(limits).map_err(|error| {
            format!(
                "GPU candidate {}: {error}. Select a compatible GPU with WGPU_ADAPTER_NAME",
                info.name
            )
        })?;
        match &mut common {
            None => common = Some(candidate),
            Some(required) => {
                required.max_buffer_size = required.max_buffer_size.min(candidate.max_buffer_size);
                required.max_storage_buffer_binding_size = required
                    .max_storage_buffer_binding_size
                    .min(candidate.max_storage_buffer_binding_size);
                required.max_texture_dimension_1d = required
                    .max_texture_dimension_1d
                    .min(candidate.max_texture_dimension_1d);
                required.max_texture_dimension_2d = required
                    .max_texture_dimension_2d
                    .min(candidate.max_texture_dimension_2d);
                required.max_texture_dimension_3d = required
                    .max_texture_dimension_3d
                    .min(candidate.max_texture_dimension_3d);
            }
        }
    }
    common.ok_or_else(|| format!("No hardware GPU matches the enabled WGPU backends and WGPU_ADAPTER_NAME={desired_name:?}"))
}

#[cfg(test)]
mod windows_tests {
    use super::*;
    use slint::wgpu_30::wgpu::{AdapterInfo, Backend, DeviceType, Limits};

    fn candidate(
        name: &str,
        device_type: DeviceType,
        buffer: u64,
        storage: u64,
    ) -> (AdapterInfo, Limits) {
        (
            AdapterInfo {
                name: name.into(),
                ..AdapterInfo::new(device_type, Backend::Dx12)
            },
            Limits {
                max_buffer_size: buffer,
                max_storage_buffer_binding_size: storage,
                max_storage_buffers_per_shader_stage: 10,
                ..Default::default()
            },
        )
    }

    #[test]
    fn dx12_candidates_request_native_limits_without_cpu_restriction() {
        let candidates = [
            candidate(
                "RTX 5070 Ti",
                DeviceType::DiscreteGpu,
                i32::MAX as u64,
                i32::MAX as u64,
            ),
            candidate(
                "AMD Radeon",
                DeviceType::IntegratedGpu,
                i32::MAX as u64,
                i32::MAX as u64,
            ),
            candidate(
                "Microsoft Basic Render Driver",
                DeviceType::Cpu,
                256 << 20,
                128 << 20,
            ),
        ];
        let limits = windows_candidate_limits(&candidates, None).unwrap();
        assert_eq!(limits.max_buffer_size, i32::MAX as u64);
        assert_eq!(limits.max_storage_buffer_binding_size, i32::MAX as u64);
        assert_eq!(limits.max_storage_buffers_per_shader_stage, 10);
    }

    #[test]
    fn mixed_candidates_use_common_limits_and_respect_name_override() {
        let mut high = candidate("RTX", DeviceType::DiscreteGpu, 8 << 30, 4 << 30);
        high.0.backend = Backend::Vulkan;
        let candidates = [
            high,
            candidate("Intel", DeviceType::IntegratedGpu, 1 << 30, 512 << 20),
        ];
        let limits = windows_candidate_limits(&candidates, None).unwrap();
        assert_eq!(limits.max_buffer_size, 1 << 30);
        assert_eq!(limits.max_storage_buffer_binding_size, 512 << 20);
        let selected = windows_candidate_limits(&candidates, Some("rTx")).unwrap();
        assert_eq!(
            selected.max_buffer_size,
            patinae_render::DEVICE_BUFFER_TARGET_BYTES
        );
        assert_eq!(
            selected.max_storage_buffer_binding_size,
            patinae_render::DEVICE_STORAGE_BINDING_TARGET_BYTES
        );
        assert!(windows_candidate_limits(&candidates, Some("missing")).is_err());
    }

    #[test]
    fn missing_hardware_or_compute_support_is_reported_before_window_creation() {
        assert!(windows_candidate_limits(&[], None).is_err());
        let cpu = candidate("CPU", DeviceType::Cpu, 1 << 30, 512 << 20);
        assert!(windows_candidate_limits(std::slice::from_ref(&cpu), None).is_err());
        assert!(windows_candidate_limits(&[cpu], Some("CPU")).is_err());
        let mut low = candidate("GPU", DeviceType::IntegratedGpu, 1 << 30, 512 << 20);
        low.1.max_storage_buffers_per_shader_stage = 8;
        assert!(windows_candidate_limits(&[low], None)
            .unwrap_err()
            .contains("requires 10"));
    }
}
