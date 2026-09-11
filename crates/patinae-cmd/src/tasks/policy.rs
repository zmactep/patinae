//! Shared result budgets used before application and during retention.

use super::*;

// These budgets reserve room for JSON escaping, identifiers, and outcome metadata.
pub(super) const MAX_LABEL_BYTES: usize = 512;
pub(super) const MAX_DIAGNOSTICS: usize = 8;
pub(super) const MAX_DIAGNOSTIC_BYTES: usize = 4 * 1024;
pub(super) const MAX_ERROR_BYTES: usize = 2048;
pub(super) const SNAPSHOT_METADATA_RESERVE: usize = 24 * 1024;

impl TaskConfig {
    /// Check that a result fits this session's retained outcome budget.
    ///
    /// # Errors
    /// Returns `result_too_large` when serialization exceeds the configured budget.
    pub fn validate_outcome(&self, outcome: &TaskOutcome) -> Result<(), TaskError> {
        match serde_json::to_vec(outcome) {
            Ok(bytes)
                if bytes.len()
                    <= self
                        .max_snapshot_bytes
                        .saturating_sub(SNAPSHOT_METADATA_RESERVE) =>
            {
                Ok(())
            }
            _ => Err(TaskError::new(
                "result_too_large",
                "task result exceeds the retained snapshot limit",
            )),
        }
    }

    pub(super) fn bound_outcome(&self, mut outcome: TaskOutcome) -> TaskOutcome {
        bound_diagnostics(&mut outcome.diagnostics, &mut outcome.diagnostics_truncated);
        match &mut outcome.status {
            TaskOutcomeStatus::Failure { error } => {
                if error.code.len() > 128 || error.message.len() > MAX_ERROR_BYTES {
                    outcome.diagnostics_truncated = true;
                }
                error.code = bounded(std::mem::take(&mut error.code), 128);
                error.message = bounded(std::mem::take(&mut error.message), MAX_ERROR_BYTES);
            }
            TaskOutcomeStatus::Cancelled { reason } => {
                if reason.len() > MAX_LABEL_BYTES {
                    outcome.diagnostics_truncated = true;
                }
                *reason = bounded(std::mem::take(reason), MAX_LABEL_BYTES);
            }
            TaskOutcomeStatus::Success { .. } => {}
        }
        if self.validate_outcome(&outcome).is_err() {
            let effects = outcome.effects;
            outcome = TaskOutcome::failure(
                "result_too_large",
                "task result exceeded the retained snapshot limit",
            );
            outcome.effects = effects;
            outcome.diagnostics_truncated = true;
        }
        outcome
    }
}

pub(super) fn bound_diagnostics(diagnostics: &mut Vec<TaskDiagnostic>, truncated: &mut bool) {
    if diagnostics.len() > MAX_DIAGNOSTICS {
        diagnostics.truncate(MAX_DIAGNOSTICS);
        *truncated = true;
    }
    for diagnostic in diagnostics.iter_mut() {
        *truncated |= diagnostic.level.len() > 64 || diagnostic.message.len() > MAX_LABEL_BYTES;
        diagnostic.level = bounded(std::mem::take(&mut diagnostic.level), 64);
        diagnostic.message = bounded(std::mem::take(&mut diagnostic.message), MAX_LABEL_BYTES);
    }
    while serde_json::to_vec(diagnostics)
        .expect("serializable diagnostics")
        .len()
        > MAX_DIAGNOSTIC_BYTES
    {
        diagnostics.pop();
        *truncated = true;
    }
}

pub(super) fn bounded(mut value: String, limit: usize) -> String {
    if value.len() > limit {
        let mut boundary = limit;
        while !value.is_char_boundary(boundary) {
            boundary -= 1;
        }
        value.truncate(boundary);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_and_retention_use_the_same_configured_budget() {
        let outcome = TaskOutcome::success(
            Some(TaskData {
                kind: "large".into(),
                schema_version: 1,
                payload: serde_json::Value::String("x".repeat(MAX_TASK_SNAPSHOT_BYTES)),
            }),
            TaskEffects::Applied,
        );
        let default = TaskConfig::default();
        assert!(default.validate_outcome(&outcome).is_err());
        let config = TaskConfig {
            max_snapshot_bytes: MAX_TASK_SNAPSHOT_BYTES * 2,
            ..default
        };
        assert!(config.validate_outcome(&outcome).is_ok());
        let tasks = TaskRunner::new(1, config, Box::new(TaskTime::default));
        let id = tasks.admit(TaskSpec::new("test", "test")).unwrap();
        tasks.finish(id, outcome);
        assert_eq!(tasks.get(id).unwrap().state, TaskState::Succeeded);
    }
}
