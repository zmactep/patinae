//! Shared parsing of PML steps. Hosts execute each step through their task runner.

use crate::error::CmdError;
use crate::tasks::{
    TaskEffects, TaskError, TaskId, TaskOutcome, TaskProgress, TaskRunner, TaskStartError,
};
use std::collections::VecDeque;

// Bound recursive includes independently of task capacity and platform scheduling.
const MAX_INCLUDE_DEPTH: usize = 100;

/// Portable PML execution state; task lifecycle remains in the session runner.
#[derive(Debug)]
pub struct ScriptExecution {
    path: String,
    lineage: Vec<String>,
    steps: Option<VecDeque<(usize, String)>>,
}

/// The next action a platform must service without blocking its host.
#[derive(Debug, PartialEq)]
pub enum ScriptAction {
    ReadSource,
    Wait,
    Execute { line: usize, command: String },
    Finished,
}

impl ScriptExecution {
    /// Prepare a source path and a separate normalized identity for cycle detection.
    ///
    /// # Errors
    /// Rejects recursive includes and excessive nesting before task admission.
    pub fn new(path: String, identity: String, parents: &[String]) -> Result<Self, TaskStartError> {
        if parents.len() >= MAX_INCLUDE_DEPTH || parents.contains(&identity) {
            return Err(TaskStartError::InvalidParent);
        }
        let mut lineage = parents.to_vec();
        lineage.push(identity);
        Ok(Self {
            path,
            lineage,
            steps: None,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn lineage(&self) -> &[String] {
        &self.lineage
    }

    /// Install source read by the platform, retaining logical line numbers.
    ///
    /// # Errors
    /// Returns a parse error without executing any commands.
    pub fn set_source(
        &mut self,
        source: &str,
        registry: &crate::CommandRegistry,
    ) -> Result<(), TaskError> {
        self.steps = Some(
            script_steps(source, registry)
                .map_err(|error| TaskError::new("invalid_script", error.to_string()))?
                .into(),
        );
        Ok(())
    }

    /// Resolve cancellation, child completion and the next executable step.
    pub fn advance(&mut self, tasks: &TaskRunner, id: TaskId, owner: &str) -> ScriptAction {
        let Ok(snapshot) = tasks.get(id) else {
            return ScriptAction::Finished;
        };
        if snapshot.state.is_terminal() {
            return ScriptAction::Finished;
        }
        if snapshot.cancel_requested {
            tasks.finish(
                id,
                TaskOutcome::cancelled("script cancellation acknowledged"),
            );
            return ScriptAction::Finished;
        }
        let Ok((pending, failed)) = tasks.children_status(id) else {
            return ScriptAction::Finished;
        };
        if pending > 0 {
            if snapshot
                .progress
                .as_ref()
                .is_none_or(|p| p.phase != "waiting_children")
            {
                let _ = tasks.progress(
                    id,
                    owner,
                    TaskProgress {
                        phase: "waiting_children".into(),
                        message: "Waiting for a script command's tasks".into(),
                        completed: None,
                        total: None,
                    },
                );
            }
            return ScriptAction::Wait;
        }
        if failed > 0 {
            tasks.finish(
                id,
                TaskOutcome::failure("child_failed", "a script command's task failed"),
            );
            return ScriptAction::Finished;
        }
        let Some(steps) = &mut self.steps else {
            return ScriptAction::ReadSource;
        };
        match steps.pop_front() {
            Some((line, command)) => ScriptAction::Execute { line, command },
            None => {
                tasks.finish(id, TaskOutcome::success(None, TaskEffects::None));
                ScriptAction::Finished
            }
        }
    }

    /// Retain a command failure with the same source location on every platform.
    pub fn complete_step(
        &self,
        tasks: &TaskRunner,
        id: TaskId,
        line: usize,
        result: Result<(), String>,
    ) -> bool {
        if let Err(error) = result {
            tasks.finish(
                id,
                TaskOutcome::failure(
                    "script_error",
                    format!("{}: line {line}: {error}", self.path),
                ),
            );
            false
        } else {
            true
        }
    }

    /// Resolve an include through the platform and quote it with the command grammar.
    ///
    /// # Errors
    /// Returns malformed-command or platform path-resolution errors.
    pub fn resolve_command(
        &self,
        command: &str,
        registry: &crate::CommandRegistry,
        resolve: impl FnOnce(&str, &str) -> Result<String, TaskError>,
    ) -> Result<String, TaskError> {
        let parsed = registry
            .parse_command(command)
            .map_err(|e| TaskError::new("invalid_script", e.to_string()))?;
        if parsed.name != "@" {
            return Ok(command.into());
        }
        let include = parsed
            .str_arg(0, "filename")
            .ok_or_else(|| TaskError::new("invalid_script", "include path is required"))?;
        let path = resolve(&self.path, include)?;
        Ok(format!(
            "run {}",
            serde_json::to_string(&path).expect("serializable path")
        ))
    }
}

/// Resolve a filesystem include relative to its containing script.
pub fn resolve_file_include(base: &str, include: &str) -> Result<String, TaskError> {
    Ok(std::path::Path::new(base)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(include)
        .to_string_lossy()
        .into_owned())
}

/// Split PML source into executable commands with logical line numbers.
///
/// Quoted semicolons and continued lines use the common command parser.
/// Hosts wait for each returned command's children before advancing.
///
/// # Errors
/// Returns a script error identifying the invalid logical line.
pub fn script_steps(
    source: &str,
    registry: &crate::CommandRegistry,
) -> Result<Vec<(usize, String)>, CmdError> {
    crate::parser::parse_steps(source, |name| registry.argument_syntax(name))
        .map(|steps| {
            steps
                .into_iter()
                .map(|(line, command)| {
                    let text = match command.raw_args() {
                        Some(args) => format!("{} {args}", command.name),
                        None => command.name,
                    };
                    (line, text)
                })
                .collect()
        })
        .map_err(|(line, error)| CmdError::script(line, error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{TaskConfig, TaskSpec, TaskState, TaskTime};

    fn runner() -> TaskRunner {
        TaskRunner::new(1, TaskConfig::default(), Box::new(TaskTime::default))
    }

    #[test]
    fn includes_preserve_quotes_and_reject_cycles_before_admission() {
        let script =
            ScriptExecution::new("/scripts/main.pml".into(), "/scripts/main.pml".into(), &[])
                .unwrap();
        let command = script
            .resolve_command(
                "@ \"child file.pml\"",
                &crate::CommandRegistry::new(),
                resolve_file_include,
            )
            .unwrap();
        assert_eq!(
            crate::parse_command(&command).unwrap().get_str(0),
            Some("/scripts/child file.pml")
        );
        assert!(
            ScriptExecution::new(script.path().into(), script.path().into(), script.lineage())
                .is_err()
        );
        assert!(ScriptExecution::new(
            "next.pml".into(),
            "next.pml".into(),
            &vec![String::new(); MAX_INCLUDE_DEPTH]
        )
        .is_err());
    }

    #[test]
    fn script_waits_for_children_and_reports_source_line_on_failure() {
        let tasks = runner();
        let id = tasks.admit(TaskSpec::new("script", "test")).unwrap();
        let mut script = ScriptExecution::new("main.pml".into(), "main.pml".into(), &[]).unwrap();
        assert_eq!(script.advance(&tasks, id, "test"), ScriptAction::ReadSource);
        script
            .set_source(
                "fetch 1crn\ninvalid_command",
                &crate::CommandRegistry::new(),
            )
            .unwrap();
        assert!(matches!(
            script.advance(&tasks, id, "test"),
            ScriptAction::Execute { line: 1, .. }
        ));
        let mut child = TaskSpec::new("fetch", "network");
        child.parent_id = Some(id);
        let child = tasks.admit(child).unwrap();
        assert_eq!(script.advance(&tasks, id, "test"), ScriptAction::Wait);
        tasks.finish(child, TaskOutcome::success(None, TaskEffects::Applied));
        assert!(matches!(
            script.advance(&tasks, id, "test"),
            ScriptAction::Execute { line: 2, .. }
        ));
        assert!(!script.complete_step(&tasks, id, 2, Err("unknown command".into())));
        let snapshot = tasks.get(id).unwrap();
        assert_eq!(snapshot.state, TaskState::Failed);
        assert_eq!(snapshot.effects, TaskEffects::Partial);
        let crate::tasks::TaskOutcomeStatus::Failure { error } = snapshot.outcome.unwrap().status
        else {
            panic!("expected failure")
        };
        assert_eq!(error.code, "script_error");
        assert!(error.message.contains("main.pml: line 2"));
    }

    #[test]
    fn cancelled_or_failed_children_prevent_the_next_step() {
        for cancel in [false, true] {
            let tasks = runner();
            let id = tasks.admit(TaskSpec::new("script", "test")).unwrap();
            let mut script =
                ScriptExecution::new("main.pml".into(), "main.pml".into(), &[]).unwrap();
            script
                .set_source("color red", &crate::CommandRegistry::new())
                .unwrap();
            if cancel {
                tasks.cancel(id).unwrap();
            } else {
                let mut child = TaskSpec::new("fetch", "network");
                child.parent_id = Some(id);
                let child = tasks.admit(child).unwrap();
                tasks.finish(child, TaskOutcome::failure("network_error", "offline"));
            }
            assert_eq!(script.advance(&tasks, id, "test"), ScriptAction::Finished);
            assert_eq!(
                tasks.get(id).unwrap().state,
                if cancel {
                    TaskState::Cancelled
                } else {
                    TaskState::Failed
                }
            );
        }
    }

    #[test]
    fn steps_split_semicolons_but_preserve_quoted_arguments() {
        let steps = super::script_steps(
            "fetch 1crn; color red\nprint \"a;b\"",
            &crate::CommandRegistry::new(),
        )
        .unwrap();
        assert_eq!(
            steps,
            vec![
                (1, "fetch 1crn".into()),
                (1, "color red".into()),
                (2, "print \"a;b\"".into())
            ]
        );
    }
}
