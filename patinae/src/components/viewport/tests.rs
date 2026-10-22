use super::*;

#[test]
fn performance_switch_accepts_indirect_validation_buffer_ceiling() {
    let mut limits = wgpu::Limits {
        max_buffer_size: u64::from(u32::MAX),
        max_storage_buffer_binding_size: PERFORMANCE_MAX_STORAGE_BUFFER_BINDING_SIZE,
        ..Default::default()
    };
    assert!(memory_policy_supported_by_device(
        RenderMemoryPolicy::performance(),
        &limits
    ));
    limits.max_storage_buffer_binding_size /= 2;
    assert!(!memory_policy_supported_by_device(
        RenderMemoryPolicy::performance(),
        &limits
    ));
    assert!(memory_policy_supported_by_device(
        RenderMemoryPolicy::balanced(),
        &limits
    ));
}

#[test]
fn recovery_outcome_requests_redraw_without_presenting_frame() {
    let outcome = ViewportRenderOutcome::recovery(Some("retry".to_string()), true);

    assert_eq!(outcome.warning.as_deref(), Some("retry"));
    assert!(outcome.request_redraw);
    assert!(!outcome.present_frame);
    assert!(outcome.image.is_none());
}
