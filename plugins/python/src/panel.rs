//! Declarative scripting panel for the embedded Python plugin.

use std::sync::{Arc, Mutex};

use patinae_plugin::prelude::*;

use crate::highlight::PythonHighlightCache;
use patinae_plugin::tasks::{TaskId, TaskSnapshot};

const MAX_OUTPUT_CHARS: usize = 24_000;

pub(crate) type ScriptPanelStateHandle = Arc<Mutex<ScriptPanelState>>;

#[derive(Debug, Default)]
pub(crate) struct ScriptPanelState {
    input: String,
    output: String,
    pub(crate) selected: Option<TaskId>,
    pub(crate) snapshot: Option<TaskSnapshot>,
    pub(crate) requests: Vec<PanelTaskRequest>,
    highlight_cache: PythonHighlightCache,
}

impl ScriptPanelState {
    fn edit_input(&mut self, input: String) {
        self.input = input;
    }

    pub(crate) fn append_output(&mut self, text: &str) {
        self.output.push_str(text);
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }
        self.trim_output();
    }

    pub(crate) fn append_error(&mut self, text: &str) {
        self.output.push_str("Error: ");
        self.output.push_str(text);
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }
        self.trim_output();
    }

    fn clear_output(&mut self) {
        self.output.clear();
    }

    fn input_highlights(&mut self) -> Vec<PanelTextHighlight> {
        self.highlight_cache.highlights_for(&self.input)
    }

    fn trim_output(&mut self) {
        if self.output.len() <= MAX_OUTPUT_CHARS {
            return;
        }

        let mut start = self.output.len() - MAX_OUTPUT_CHARS;
        while !self.output.is_char_boundary(start) {
            start += 1;
        }
        self.output.drain(..start);
        self.output.insert_str(0, "[output truncated]\n");
    }
}

#[derive(Debug)]
pub(crate) enum PanelTaskRequest {
    Run(String),
    Cancel(TaskId),
}

pub(crate) fn shared_panel_state() -> ScriptPanelStateHandle {
    Arc::new(Mutex::new(ScriptPanelState::default()))
}

pub(crate) struct PythonScriptPanel {
    state: ScriptPanelStateHandle,
}

impl PythonScriptPanel {
    pub(crate) fn new(state: ScriptPanelStateHandle) -> Self {
        Self { state }
    }

    fn run_script(&self) {
        let mut state = self.state.lock().unwrap();
        let code = state.input.clone();
        if code.trim().is_empty() {
            state.append_output("No script to run.");
        } else {
            state.requests.push(PanelTaskRequest::Run(code));
        }
    }

    fn stop_script(&self) {
        let mut state = self.state.lock().unwrap();
        if let Some(id) = state.selected {
            state.requests.push(PanelTaskRequest::Cancel(id));
        }
    }
}

impl PluginPanel for PythonScriptPanel {
    fn descriptor(&self) -> PanelDescriptor {
        PanelDescriptor::bottom("python_scripting", "Python")
            .icon("Py")
            .default_visible(false)
    }

    fn runtime_requirements(&self) -> PanelRuntimeRequirements {
        PanelRuntimeRequirements::NONE
    }

    fn snapshot(&mut self, _ctx: &SharedContext<'_>) -> PanelSnapshot {
        let mut state = self.state.lock().unwrap();
        let active = state
            .snapshot
            .as_ref()
            .is_some_and(|task| !task.state.is_terminal());
        let has_output = !state.output.is_empty();
        let can_cancel = state
            .snapshot
            .as_ref()
            .is_some_and(|task| task.cancellable && !task.cancel_requested);
        let input = state.input.clone();
        let input_highlights = state.input_highlights();
        let mut controls = vec![
            PanelControl::ButtonRow {
                id: "toolbar".into(),
                buttons: vec![
                    PanelButton::new("run", "Run script", "run", true).enabled(!active),
                    PanelButton::new("stop", "Stop", "stop", false).enabled(active && can_cancel),
                    PanelButton::new("clear_output", "Clear output", "clear", false)
                        .enabled(has_output),
                ],
            },
            PanelControl::Row(
                PanelRow::new(
                    "script_output",
                    vec![
                        PanelControlNode::new(PanelControl::TextArea(
                            PanelTextArea::new(
                                "script",
                                "",
                                input,
                                "print('hello from Patinae')",
                                5,
                                false,
                            )
                            .with_highlights(input_highlights),
                        ))
                        .grow(1.0),
                        PanelControlNode::new(PanelControl::TextArea(PanelTextArea::new(
                            "output",
                            "",
                            state.output.clone(),
                            "Output will appear here.",
                            5,
                            true,
                        )))
                        .grow(1.0),
                    ],
                )
                .gap(8.0),
            ),
        ];

        if let Some(task) = &state.snapshot {
            controls.push(PanelControl::Text {
                id: "status".into(),
                text: format!(
                    "{:?}: {}",
                    task.state,
                    task.progress
                        .as_ref()
                        .map_or("", |progress| progress.message.as_str())
                ),
            });
        }

        PanelSnapshot::new(controls)
    }

    fn handle_event(
        &mut self,
        event: PanelEvent,
        _ctx: &SharedContext<'_>,
        _bus: &mut MessageBus,
    ) -> Vec<PanelAction> {
        match event.control_id.as_str() {
            "script" => {
                if let PanelValue::Text(text) = event.value {
                    self.state.lock().unwrap().edit_input(text);
                }
            }
            "run" => self.run_script(),
            "stop" => self.stop_script(),
            "clear_output" => self.state.lock().unwrap().clear_output(),
            _ => {}
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_panel_uses_lightweight_runtime_input() {
        let panel = PythonScriptPanel::new(shared_panel_state());

        assert!(panel.runtime_requirements().is_empty());
    }

    #[test]
    fn edit_input_updates_cached_highlights_source() {
        let mut state = ScriptPanelState::default();
        state.edit_input("def f():\n    return 'hi'\n".into());

        let first = state.input_highlights();
        let second = state.input_highlights();

        assert!(!first.is_empty());
        assert_eq!(first, second);
    }

    #[test]
    fn output_can_be_cleared() {
        let mut state = ScriptPanelState::default();

        state.append_output("hello");
        assert!(!state.output.is_empty());

        state.clear_output();
        assert!(state.output.is_empty());
    }
}
