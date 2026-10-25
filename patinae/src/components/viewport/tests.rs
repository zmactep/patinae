use super::*;
use patinae_render::{select_render_memory_policy, RenderMemorySelectionInput};

#[test]
fn profile_switches_restore_features_with_small_resource_limits() {
    use patinae_settings::RenderMemoryProfileSetting as Setting;
    let mut settings = Settings::default();
    let auto = select_render_memory_policy(
        RenderMemorySelectionInput::from_wgpu(
            &wgpu::AdapterInfo::new(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Dx12),
            &wgpu::Limits::default(),
            false,
        ),
        Some(patinae_render::NativeGpuMemoryBudget {
            budget_bytes: 16 << 30,
            usage_bytes: 0,
            source: patinae_render::NativeMemoryBudgetSource::DxgiLocal,
        }),
        None,
    )
    .policy;
    assert_eq!(auto.profile, RenderMemoryProfile::Performance);
    for (setting, expected, full_features) in [
        (Setting::Performance, RenderMemoryProfile::Performance, true),
        (Setting::Balanced, RenderMemoryProfile::Balanced, true),
        (Setting::Lite, RenderMemoryProfile::Lite, false),
        (Setting::Balanced, RenderMemoryProfile::Balanced, true),
        (Setting::Performance, RenderMemoryProfile::Performance, true),
    ] {
        settings.renderer.memory_profile = setting;
        let policy = render_memory_policy_from_settings(&settings, auto);
        assert_eq!(policy.profile, expected);
        assert_eq!(policy.overlays.selection_enabled, full_features);
        assert_eq!(policy.picking.reprojection_enabled, full_features);
        assert_eq!(policy.postprocess.fxaa_enabled, full_features);
    }
}

#[test]
fn recovery_outcome_requests_redraw_without_presenting_frame() {
    let outcome = ViewportRenderOutcome::recovery(Some("retry".to_string()), true);

    assert_eq!(outcome.warning.as_deref(), Some("retry"));
    assert!(outcome.request_redraw);
    assert!(!outcome.present_frame);
    assert!(outcome.image.is_none());
}
