use patinae_framework::plugin_ui::{PanelEventKind, PanelValue};

pub(crate) fn panel_event_payload(
    kind: &str,
    text: &str,
    number: f32,
    boolean: bool,
) -> Option<(PanelEventKind, PanelValue)> {
    match kind {
        "click" => Some((PanelEventKind::Click, PanelValue::None)),
        "toggle" => Some((PanelEventKind::Toggle, PanelValue::Bool(boolean))),
        "number" => Some((PanelEventKind::NumberChange, PanelValue::Number(number))),
        "select" => Some((PanelEventKind::Select, PanelValue::Text(text.to_string()))),
        "text-edit" => Some((PanelEventKind::TextEdit, PanelValue::Text(text.to_string()))),
        "text-area-edit" => Some((
            PanelEventKind::TextAreaEdit,
            PanelValue::Text(text.to_string()),
        )),
        "text-commit" | "text" => Some((
            PanelEventKind::TextCommit,
            PanelValue::Text(text.to_string()),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_edit_event_is_typed_and_transient() {
        let (kind, value) = panel_event_payload("text-edit", "print('hi')", 0.0, false).unwrap();

        assert_eq!(kind, PanelEventKind::TextEdit);
        assert!(!kind.refreshes_snapshot());
        assert!(matches!(value, PanelValue::Text(text) if text == "print('hi')"));
    }

    #[test]
    fn text_area_edit_event_refreshes_panel_snapshots() {
        let (kind, value) =
            panel_event_payload("text-area-edit", "print('hi')", 0.0, false).unwrap();

        assert_eq!(kind, PanelEventKind::TextAreaEdit);
        assert!(kind.refreshes_snapshot());
        assert!(matches!(value, PanelValue::Text(text) if text == "print('hi')"));
    }

    #[test]
    fn panel_events_preserve_kind_and_payload() {
        for (event, expected) in [
            ("click", PanelEventKind::Click),
            ("toggle", PanelEventKind::Toggle),
            ("number", PanelEventKind::NumberChange),
            ("select", PanelEventKind::Select),
            ("text-edit", PanelEventKind::TextEdit),
            ("text-area-edit", PanelEventKind::TextAreaEdit),
            ("text-commit", PanelEventKind::TextCommit),
            ("text", PanelEventKind::TextCommit),
        ] {
            let (kind, value) = panel_event_payload(event, "payload", 3.25, true).unwrap();
            assert_eq!(kind, expected, "{event}");
            let matches = match event {
                "click" => matches!(value, PanelValue::None),
                "toggle" => matches!(value, PanelValue::Bool(true)),
                "number" => matches!(value, PanelValue::Number(n) if n == 3.25),
                _ => matches!(value, PanelValue::Text(text) if text == "payload"),
            };
            assert!(matches, "{event} payload");
        }
    }

    #[test]
    fn unknown_panel_event_kind_is_ignored() {
        assert!(panel_event_payload("mystery", "", 0.0, false).is_none());
    }
}
