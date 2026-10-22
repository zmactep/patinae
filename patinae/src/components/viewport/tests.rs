use super::*;

#[test]
fn recovery_outcome_requests_redraw_without_presenting_frame() {
    let outcome = ViewportRenderOutcome::recovery(Some("retry".to_string()), true);

    assert_eq!(outcome.warning.as_deref(), Some("retry"));
    assert!(outcome.request_redraw);
    assert!(!outcome.present_frame);
    assert!(outcome.image.is_none());
}
