//! Authoritative lifecycle and retention of session tasks.

use super::policy::*;
use super::*;
use std::{cell::RefCell, collections::BTreeMap};

struct TaskRecord {
    snapshot: TaskSnapshot,
    created_at_monotonic_ms: u64,
    owner: String,
    scene_epoch: Option<u64>,
    cancellable: bool,
    finished_at: Option<(u64, u64)>,
    body_outcome: Option<TaskOutcome>,
    pending_children: usize,
    failed_children: u64,
    child_failure_policy: ChildFailurePolicy,
    silent: bool,
    child_diagnostics: Vec<TaskDiagnostic>,
}

struct TaskRecords {
    next_sequence: u64,
    completion_sequence: u64,
    tasks: BTreeMap<TaskId, TaskRecord>,
    changes: BTreeMap<TaskId, TaskChanged>,
}

/// Authoritative task records owned by one session's host thread.
///
/// The runner is movable between threads, but callers must serialize access.
/// Its short internal borrows never span executor work or result application.
pub struct TaskRunner {
    instance: u128,
    config: TaskConfig,
    clock: Box<dyn TaskClock + Send>,
    records: RefCell<TaskRecords>,
}

impl TaskRunner {
    /// Construct a session registry using host-issued identity and platform clocks.
    ///
    /// # Panics
    /// Panics if the response budget cannot hold metadata or page size is zero.
    pub fn new(instance: u128, config: TaskConfig, clock: Box<dyn TaskClock + Send>) -> Self {
        assert!(config.max_snapshot_bytes >= MAX_TASK_SNAPSHOT_BYTES);
        assert!(config.max_page_items > 0);
        Self {
            instance,
            config,
            clock,
            records: RefCell::new(TaskRecords {
                next_sequence: 1,
                completion_sequence: 0,
                tasks: BTreeMap::new(),
                changes: BTreeMap::new(),
            }),
        }
    }

    /// Read the limits used by admission, queries, and capabilities.
    pub fn config(&self) -> &TaskConfig {
        &self.config
    }

    /// Accept work atomically before allocating executor resources.
    ///
    /// # Errors
    /// Rejects exhausted capacity, exhausted identities, or unavailable parents.
    pub fn admit(&self, spec: TaskSpec) -> Result<TaskId, TaskStartError> {
        let now = self.clock.now();
        let mut records = self.records.borrow_mut();
        self.prune(&mut records, now);
        if records
            .tasks
            .values()
            .filter(|r| !r.snapshot.state.is_terminal())
            .count()
            >= self.config.max_active
        {
            return Err(TaskStartError::Busy);
        }
        if let Some(parent_id) = spec.parent_id {
            let valid = records.tasks.get(&parent_id).is_some_and(|parent| {
                !parent.snapshot.state.is_terminal()
                    && !parent.snapshot.cancel_requested
                    && parent.body_outcome.is_none()
            });
            if !valid {
                return Err(TaskStartError::InvalidParent);
            }
        }
        let sequence = records.next_sequence;
        let silent = spec.silent || spec.parent_id.is_some_and(|id| records.tasks[&id].silent);
        let next = sequence.checked_add(1).ok_or(TaskStartError::IdExhausted)?;
        let id = TaskId::new(self.instance, sequence);
        let snapshot = TaskSnapshot {
            id,
            parent_id: spec.parent_id,
            kind: bounded(spec.kind, MAX_LABEL_BYTES),
            origin: bounded(spec.origin, MAX_LABEL_BYTES),
            state: TaskState::Queued,
            revision: 1,
            progress: Some(TaskProgress {
                phase: "queued".into(),
                message: bounded(spec.message, MAX_LABEL_BYTES),
                completed: None,
                total: None,
            }),
            cancel_requested: false,
            cancellable: spec.cancellable,
            created_at_ms: now.unix_ms,
            updated_at_ms: now.unix_ms,
            effects: TaskEffects::None,
            diagnostics: Vec::new(),
            diagnostics_truncated: false,
            outcome: None,
        };
        records.next_sequence = next;
        records.tasks.insert(
            id,
            TaskRecord {
                snapshot,
                created_at_monotonic_ms: now.monotonic_ms,
                owner: spec.owner,
                scene_epoch: spec.scene_epoch,
                cancellable: spec.cancellable,
                finished_at: None,
                body_outcome: None,
                pending_children: 0,
                failed_children: 0,
                child_failure_policy: spec.child_failure_policy,
                silent,
                child_diagnostics: Vec::new(),
            },
        );
        Self::changed(&mut records, id);
        if let Some(parent_id) = spec.parent_id {
            records
                .tasks
                .get_mut(&parent_id)
                .expect("validated parent")
                .pending_children += 1;
            Self::touch(&mut records, parent_id, now);
        }
        Ok(id)
    }

    /// Mark an executor's accepted work as running.
    ///
    /// # Errors
    /// Returns a lookup or executor-ownership error.
    pub fn started(&self, id: TaskId, owner: &str) -> Result<bool, TaskError> {
        let mut records = self.records.borrow_mut();
        self.check_owner(&records, id, owner)?;
        let record = records.tasks.get_mut(&id).expect("checked task");
        if record.snapshot.state != TaskState::Queued || record.body_outcome.is_some() {
            return Ok(false);
        }
        record.snapshot.state = TaskState::Running;
        if let Some(progress) = &mut record.snapshot.progress {
            progress.phase = "running".into();
        }
        Self::touch(&mut records, id, self.clock.now());
        Ok(true)
    }

    /// Retain bounded progress from the assigned executor.
    ///
    /// # Errors
    /// Returns a lookup or executor-ownership error.
    pub fn progress(
        &self,
        id: TaskId,
        owner: &str,
        mut progress: TaskProgress,
    ) -> Result<bool, TaskError> {
        let mut records = self.records.borrow_mut();
        self.check_owner(&records, id, owner)?;
        let record = records.tasks.get_mut(&id).expect("checked task");
        if !Self::accepts_events(record) {
            return Ok(false);
        }
        progress.phase = bounded(progress.phase, MAX_LABEL_BYTES);
        progress.message = bounded(progress.message, MAX_LABEL_BYTES);
        if record.snapshot.progress.as_ref() == Some(&progress) {
            return Ok(false);
        }
        record.snapshot.progress = Some(progress);
        Self::touch(&mut records, id, self.clock.now());
        Ok(true)
    }

    /// Retain bounded executor output in the task's current snapshot.
    ///
    /// # Errors
    /// Returns a lookup or executor-ownership error.
    pub fn output(
        &self,
        id: TaskId,
        owner: &str,
        diagnostic: TaskDiagnostic,
    ) -> Result<bool, TaskError> {
        let mut records = self.records.borrow_mut();
        self.check_owner(&records, id, owner)?;
        let record = records.tasks.get_mut(&id).expect("checked task");
        if !Self::accepts_events(record) {
            return Ok(false);
        }
        Self::append_diagnostic(&mut record.snapshot, diagnostic);
        Self::touch(&mut records, id, self.clock.now());
        Ok(true)
    }

    /// Verify streaming mutation ownership, cancellation, and scene lifetime.
    ///
    /// # Errors
    /// Rejects wrong executors, unavailable tasks, cancellation, or stale scenes.
    pub fn can_apply_effect(
        &self,
        id: TaskId,
        owner: &str,
        scene_epoch: u64,
    ) -> Result<(), TaskError> {
        let records = self.records.borrow();
        self.check_owner(&records, id, owner)?;
        let record = &records.tasks[&id];
        if !Self::accepts_events(record) {
            return Err(TaskError::new(
                "task_finished",
                "task no longer accepts effects",
            ));
        }
        if record.snapshot.cancel_requested {
            return Err(TaskError::new(
                "cancelled",
                "task cancellation was requested",
            ));
        }
        if record.scene_epoch.is_some_and(|epoch| epoch != scene_epoch) {
            return Err(TaskError::new(
                "stale_context",
                "scene was cleared or replaced",
            ));
        }
        Ok(())
    }

    /// Record acknowledged effects after a host mutation has been applied.
    ///
    /// # Errors
    /// Returns a lookup or executor-ownership error.
    pub fn record_effects(
        &self,
        id: TaskId,
        owner: &str,
        effects: TaskEffects,
    ) -> Result<bool, TaskError> {
        let mut records = self.records.borrow_mut();
        self.check_owner(&records, id, owner)?;
        let record = records.tasks.get_mut(&id).expect("checked task");
        if record.snapshot.state.is_terminal() || record.body_outcome.is_some() {
            return Ok(false);
        }
        let merged = merge_effects(record.snapshot.effects, effects);
        if merged == record.snapshot.effects {
            return Ok(false);
        }
        record.snapshot.effects = merged;
        Self::touch(&mut records, id, self.clock.now());
        Ok(true)
    }

    /// Check the final result before applying its mandatory effects.
    ///
    /// Cancellation is acknowledged here only because the executor has returned.
    pub fn begin_apply(&self, id: TaskId, scene_epoch: u64) -> bool {
        let now = self.clock.now();
        let mut records = self.records.borrow_mut();
        let Some(record) = records.tasks.get_mut(&id) else {
            return false;
        };
        if !Self::accepts_events(record) {
            return false;
        }
        if record.snapshot.cancel_requested {
            self.finish_record(&mut records, id, TaskOutcome::cancelled("requested"), now);
            return false;
        }
        if record.scene_epoch.is_some_and(|epoch| epoch != scene_epoch) {
            self.finish_record(
                &mut records,
                id,
                TaskOutcome::failure("stale_context", "scene was cleared or replaced"),
                now,
            );
            return false;
        }
        record.snapshot.state = TaskState::Applying;
        record.snapshot.cancellable = false;
        Self::touch(&mut records, id, now);
        true
    }

    /// Record body completion, waiting for accepted children before terminalization.
    pub fn finish(&self, id: TaskId, outcome: TaskOutcome) -> bool {
        self.finish_record(
            &mut self.records.borrow_mut(),
            id,
            outcome,
            self.clock.now(),
        )
    }

    /// Accept completion from the assigned executor after host-side application.
    ///
    /// # Errors
    /// Returns a lookup or executor-ownership error.
    pub fn finish_owned(
        &self,
        id: TaskId,
        owner: &str,
        outcome: TaskOutcome,
    ) -> Result<bool, TaskError> {
        let mut records = self.records.borrow_mut();
        self.check_owner(&records, id, owner)?;
        Ok(self.finish_record(&mut records, id, outcome, self.clock.now()))
    }

    /// Read an atomic snapshot, including retained output and terminal outcome.
    ///
    /// # Errors
    /// Distinguishes another instance, an unissued identity, and expired history.
    pub fn get(&self, id: TaskId) -> Result<TaskSnapshot, TaskLookupError> {
        let mut records = self.records.borrow_mut();
        self.prune(&mut records, self.clock.now());
        self.lookup(&records, id)?;
        Ok(records.tasks[&id].snapshot.clone())
    }

    /// Read the inherited REPL presentation policy without changing captured output.
    ///
    /// # Errors
    /// Returns a lookup error for unavailable tasks.
    pub fn is_silent(&self, id: TaskId) -> Result<bool, TaskLookupError> {
        let records = self.records.borrow();
        self.lookup(&records, id)?;
        Ok(records.tasks[&id].silent)
    }

    /// Measure admission-to-completion time, including queued work and child tasks.
    ///
    /// Uses the host's monotonic clock and freezes at terminal completion.
    ///
    /// # Errors
    /// Returns a lookup error when the task does not exist or has expired.
    pub fn elapsed_ms(&self, id: TaskId) -> Result<u64, TaskLookupError> {
        let now = self.clock.now();
        let mut records = self.records.borrow_mut();
        self.prune(&mut records, now);
        self.lookup(&records, id)?;
        let record = &records.tasks[&id];
        let end = record
            .finished_at
            .map_or(now.monotonic_ms, |(time, _)| time);
        Ok(end.saturating_sub(record.created_at_monotonic_ms))
    }

    /// Enumerate bounded summaries in stable issuance order.
    ///
    /// # Errors
    /// Rejects cursors from another instance or beyond issued identities.
    pub fn list(&self, request: &TaskListRequest) -> Result<TaskListPage, TaskLookupError> {
        let mut records = self.records.borrow_mut();
        self.prune(&mut records, self.clock.now());
        if let Some(after) = request.after {
            self.check_identity(&records, after)?;
        }
        let limit = request
            .limit
            .unwrap_or(self.config.max_page_items)
            .clamp(1, self.config.max_page_items);
        let mut tasks = Vec::new();
        let mut bytes = 128;
        let mut next_cursor = None;
        for record in records.tasks.values().filter(|record| {
            request.after.is_none_or(|after| record.snapshot.id > after)
                && (!request.active_only || !record.snapshot.state.is_terminal())
                && request
                    .origin
                    .as_ref()
                    .is_none_or(|origin| origin == &record.snapshot.origin)
        }) {
            let summary = TaskSummary::from(&record.snapshot);
            let size = serde_json::to_vec(&summary)
                .expect("serializable summary")
                .len()
                + 1;
            if tasks.len() == limit || bytes + size > self.config.max_snapshot_bytes {
                next_cursor = tasks.last().map(|task: &TaskSummary| task.id);
                break;
            }
            bytes += size;
            tasks.push(summary);
        }
        Ok(TaskListPage { tasks, next_cursor })
    }

    /// Request cancellation for a task and every cancellable active descendant.
    ///
    /// # Errors
    /// Returns the same lookup errors as `get`.
    pub fn cancel(&self, id: TaskId) -> Result<TaskCancelReply, TaskLookupError> {
        let now = self.clock.now();
        let mut records = self.records.borrow_mut();
        self.prune(&mut records, now);
        self.lookup(&records, id)?;
        let root = &records.tasks[&id];
        if root.snapshot.state.is_terminal() || root.snapshot.state == TaskState::Applying {
            return Ok(TaskCancelReply::TooLate);
        }
        if !root.snapshot.cancellable {
            return Ok(TaskCancelReply::Unsupported);
        }
        let reply = if root.snapshot.cancel_requested {
            TaskCancelReply::AlreadyRequested
        } else {
            TaskCancelReply::Requested
        };
        let targets: Vec<_> = records
            .tasks
            .keys()
            .copied()
            .filter(|candidate| Self::descends_from(&records, *candidate, id))
            .collect();
        for target in targets {
            let record = records.tasks.get_mut(&target).expect("retained descendant");
            if !record.snapshot.state.is_terminal()
                && record.snapshot.state != TaskState::Applying
                && record.snapshot.cancellable
                && !record.snapshot.cancel_requested
            {
                record.snapshot.cancel_requested = true;
                Self::touch(&mut records, target, now);
            }
        }
        Ok(reply)
    }

    /// Read active tasks whose executors still need cancellation acknowledgement.
    pub fn cancellation_targets(&self) -> Vec<(TaskId, String)> {
        self.records
            .borrow()
            .tasks
            .values()
            .filter(|record| {
                record.snapshot.cancel_requested
                    && !record.snapshot.state.is_terminal()
                    && record.body_outcome.is_none()
            })
            .map(|record| (record.snapshot.id, record.owner.clone()))
            .collect()
    }

    /// Return active snapshots for host scheduling and presentation.
    pub fn active_snapshots(&self) -> Vec<TaskSnapshot> {
        self.records
            .borrow()
            .tasks
            .values()
            .filter(|record| !record.snapshot.state.is_terminal())
            .map(|record| record.snapshot.clone())
            .collect()
    }

    /// Count tasks whose own work or descendants have not finished.
    pub fn pending_count(&self) -> usize {
        self.records
            .borrow()
            .tasks
            .values()
            .filter(|record| !record.snapshot.state.is_terminal())
            .count()
    }

    /// Read the executor identity selected by the host at admission.
    ///
    /// # Errors
    /// Returns the same identity errors as `get`.
    pub fn owner(&self, id: TaskId) -> Result<String, TaskLookupError> {
        let records = self.records.borrow();
        self.lookup(&records, id)?;
        Ok(records.tasks[&id].owner.clone())
    }

    /// Check executor ownership without exposing the record.
    pub fn is_owned_by(&self, id: TaskId, owner: &str) -> bool {
        self.records
            .borrow()
            .tasks
            .get(&id)
            .is_some_and(|record| record.owner == owner)
    }

    /// Fail unfinished executor bodies after confirmed executor loss.
    ///
    /// Descendants on other executors receive cancellation requests. Their
    /// acknowledgements still determine when their parent can become terminal.
    pub fn fail_owner(&self, owner: &str) -> usize {
        let targets: Vec<_> = self
            .records
            .borrow()
            .tasks
            .values()
            .filter(|record| {
                record.owner == owner
                    && !record.snapshot.state.is_terminal()
                    && record.body_outcome.is_none()
            })
            .map(|record| record.snapshot.id)
            .collect();
        for id in &targets {
            // Executor loss must stop descendants even when the lost body
            // itself cannot be cancelled (for example, while applying).
            {
                let now = self.clock.now();
                let mut records = self.records.borrow_mut();
                let descendants: Vec<_> = records
                    .tasks
                    .keys()
                    .copied()
                    .filter(|candidate| {
                        candidate != id && Self::descends_from(&records, *candidate, *id)
                    })
                    .collect();
                for child in descendants {
                    let record = records.tasks.get_mut(&child).expect("retained descendant");
                    if !record.snapshot.state.is_terminal()
                        && record.snapshot.state != TaskState::Applying
                        && record.snapshot.cancellable
                        && !record.snapshot.cancel_requested
                    {
                        record.snapshot.cancel_requested = true;
                        Self::touch(&mut records, child, now);
                    }
                }
            }
            let mut outcome =
                TaskOutcome::failure("executor_lost", "task executor disconnected or stopped");
            outcome.effects = TaskEffects::Unknown;
            self.finish(*id, outcome);
        }
        targets.len()
    }

    /// Read retained child counters without consulting evictable child history.
    pub fn children_status(&self, parent: TaskId) -> Result<(usize, u64), TaskLookupError> {
        let records = self.records.borrow();
        self.lookup(&records, parent)?;
        let record = &records.tasks[&parent];
        Ok((record.pending_children, record.failed_children))
    }

    /// Validate an adapter's wait before it blocks a sequential executor.
    ///
    /// # Errors
    /// Returns lookup errors or `would_deadlock` for self, ancestor, or blocked work.
    pub fn validate_wait(
        &self,
        waiter: Option<TaskId>,
        target: TaskId,
        serial_owner: Option<&str>,
    ) -> Result<(), TaskError> {
        let mut records = self.records.borrow_mut();
        self.prune(&mut records, self.clock.now());
        self.lookup(&records, target).map_err(Self::lookup_error)?;
        let Some(waiter) = waiter else { return Ok(()) };
        self.lookup(&records, waiter).map_err(Self::lookup_error)?;
        if records.tasks[&target].snapshot.state.is_terminal() {
            return Ok(());
        }
        let ancestor = Self::descends_from(&records, waiter, target);
        let blocked = serial_owner.is_some_and(|owner| {
            records.tasks.values().any(|record| {
                record.owner == owner
                    && record.body_outcome.is_none()
                    && !record.snapshot.state.is_terminal()
                    && Self::descends_from(&records, record.snapshot.id, target)
            })
        });
        if ancestor || blocked {
            return Err(TaskError::new(
                "would_deadlock",
                "task cannot wait for itself, an ancestor, or work blocked by its executor",
            ));
        }
        Ok(())
    }

    /// Drain coalesced invalidation hints after the corresponding updates.
    pub fn take_changes(&self) -> Vec<TaskChanged> {
        std::mem::take(&mut self.records.borrow_mut().changes)
            .into_values()
            .collect()
    }

    fn accepts_events(record: &TaskRecord) -> bool {
        !record.snapshot.state.is_terminal()
            && record.snapshot.state != TaskState::Applying
            && record.body_outcome.is_none()
    }

    fn check_identity(&self, records: &TaskRecords, id: TaskId) -> Result<(), TaskLookupError> {
        if id.instance() != self.instance {
            return Err(TaskLookupError::WrongInstance);
        }
        if id.sequence() == 0 || id.sequence() >= records.next_sequence {
            return Err(TaskLookupError::NotFound);
        }
        Ok(())
    }

    fn lookup(&self, records: &TaskRecords, id: TaskId) -> Result<(), TaskLookupError> {
        self.check_identity(records, id)?;
        if !records.tasks.contains_key(&id) {
            return Err(TaskLookupError::Expired);
        }
        Ok(())
    }

    fn lookup_error(error: TaskLookupError) -> TaskError {
        TaskError::new(error.to_string(), "task identity is unavailable")
    }

    fn check_owner(&self, records: &TaskRecords, id: TaskId, owner: &str) -> Result<(), TaskError> {
        self.lookup(records, id).map_err(Self::lookup_error)?;
        if records.tasks[&id].owner != owner {
            return Err(TaskError::new(
                "wrong_executor",
                "task belongs to another executor",
            ));
        }
        Ok(())
    }

    fn descends_from(records: &TaskRecords, mut candidate: TaskId, ancestor: TaskId) -> bool {
        loop {
            if candidate == ancestor {
                return true;
            }
            let Some(parent) = records
                .tasks
                .get(&candidate)
                .and_then(|record| record.snapshot.parent_id)
            else {
                return false;
            };
            candidate = parent;
        }
    }

    fn append_diagnostic(snapshot: &mut TaskSnapshot, diagnostic: TaskDiagnostic) {
        snapshot.diagnostics.push(diagnostic);
        bound_diagnostics(
            &mut snapshot.diagnostics,
            &mut snapshot.diagnostics_truncated,
        );
    }

    fn changed(records: &mut TaskRecords, id: TaskId) {
        let snapshot = &records.tasks[&id].snapshot;
        records.changes.insert(
            id,
            TaskChanged {
                id,
                revision: snapshot.revision,
                state: snapshot.state,
            },
        );
    }

    fn touch(records: &mut TaskRecords, id: TaskId, now: TaskTime) {
        let snapshot = &mut records
            .tasks
            .get_mut(&id)
            .expect("registered task")
            .snapshot;
        snapshot.revision = snapshot.revision.saturating_add(1);
        snapshot.updated_at_ms = now.unix_ms;
        Self::changed(records, id);
    }

    fn finish_record(
        &self,
        records: &mut TaskRecords,
        id: TaskId,
        mut outcome: TaskOutcome,
        now: TaskTime,
    ) -> bool {
        let Some(record) = records.tasks.get_mut(&id) else {
            return false;
        };
        if record.snapshot.state.is_terminal() || record.body_outcome.is_some() {
            return false;
        }
        outcome = self.config.bound_outcome(outcome);
        for diagnostic in std::mem::take(&mut outcome.diagnostics) {
            Self::append_diagnostic(&mut record.snapshot, diagnostic);
        }
        record.snapshot.diagnostics_truncated |= outcome.diagnostics_truncated;
        record.snapshot.effects = merge_effects(record.snapshot.effects, outcome.effects);
        record.body_outcome = Some(outcome);
        if record.pending_children > 0 {
            record.snapshot.state = TaskState::Running;
            record.snapshot.cancellable = record.cancellable;
            record.snapshot.progress = Some(TaskProgress {
                phase: "waiting_children".into(),
                message: "Waiting for child tasks".into(),
                completed: None,
                total: None,
            });
            Self::touch(records, id, now);
        } else {
            self.complete_tree(records, id, now);
        }
        self.prune(records, now);
        true
    }

    fn complete_tree(&self, records: &mut TaskRecords, mut id: TaskId, now: TaskTime) {
        loop {
            let record = records
                .tasks
                .get_mut(&id)
                .expect("completing retained task");
            let mut outcome = record.body_outcome.take().expect("completed body");
            if matches!(outcome.status, TaskOutcomeStatus::Success { .. }) {
                if record.snapshot.cancel_requested {
                    outcome.status = TaskOutcomeStatus::Cancelled {
                        reason: "requested".into(),
                    };
                } else if record.failed_children > 0
                    && record.child_failure_policy == ChildFailurePolicy::Propagate
                {
                    outcome.status = TaskOutcomeStatus::Failure {
                        error: TaskError::new(
                            "child_failed",
                            format!(
                                "{} child task(s) failed or were cancelled",
                                record.failed_children
                            ),
                        ),
                    };
                }
            }
            for diagnostic in std::mem::take(&mut record.child_diagnostics) {
                Self::append_diagnostic(&mut record.snapshot, diagnostic);
            }
            outcome.effects = record.snapshot.effects;
            if !matches!(outcome.status, TaskOutcomeStatus::Success { .. })
                && outcome.effects == TaskEffects::Applied
            {
                outcome.effects = TaskEffects::Partial;
            }
            outcome.diagnostics = record.snapshot.diagnostics.clone();
            outcome.diagnostics_truncated = record.snapshot.diagnostics_truncated;
            outcome = self.config.bound_outcome(outcome);
            record.snapshot.effects = outcome.effects;
            record.snapshot.diagnostics_truncated = outcome.diagnostics_truncated;
            record.snapshot.state = outcome.state();
            record.snapshot.cancellable = false;
            let parent_id = record.snapshot.parent_id;
            let effects = outcome.effects;
            let child_error = match &outcome.status {
                TaskOutcomeStatus::Success { .. } => None,
                TaskOutcomeStatus::Failure { error } => Some(format!("{id}: {error}")),
                TaskOutcomeStatus::Cancelled { reason } => {
                    Some(format!("{id}: cancelled: {reason}"))
                }
            };
            record.snapshot.outcome = Some(outcome);
            // At most one completion exists per issued ID, so this cannot overflow first.
            records.completion_sequence = records
                .completion_sequence
                .checked_add(1)
                .expect("completion count bounded by issued task identities");
            record.finished_at = Some((now.monotonic_ms, records.completion_sequence));
            Self::touch(records, id, now);
            let Some(parent_id) = parent_id else { break };
            let parent = records
                .tasks
                .get_mut(&parent_id)
                .expect("active parent cannot expire");
            parent.pending_children -= 1;
            parent.snapshot.effects = merge_effects(parent.snapshot.effects, effects);
            if let Some(message) = child_error {
                parent.failed_children = parent.failed_children.saturating_add(1);
                if parent.child_diagnostics.len() < MAX_DIAGNOSTICS {
                    parent.child_diagnostics.push(TaskDiagnostic {
                        level: "error".into(),
                        message: bounded(message, MAX_LABEL_BYTES),
                    });
                } else {
                    parent.snapshot.diagnostics_truncated = true;
                }
            }
            let complete = parent.pending_children == 0 && parent.body_outcome.is_some();
            Self::touch(records, parent_id, now);
            if !complete {
                break;
            }
            id = parent_id;
        }
    }

    fn prune(&self, records: &mut TaskRecords, now: TaskTime) {
        records.tasks.retain(|_, record| {
            record.finished_at.is_none_or(|(finished, _)| {
                now.monotonic_ms.saturating_sub(finished) < self.config.terminal_ttl_ms
            })
        });
        let mut terminal: Vec<_> = records
            .tasks
            .iter()
            .filter_map(|(id, record)| record.finished_at.map(|time| (*id, time)))
            .collect();
        terminal.sort_by_key(|(_, time)| *time);
        let excess = terminal.len().saturating_sub(self.config.max_terminal);
        for (id, _) in terminal.into_iter().take(excess) {
            records.tasks.remove(&id);
        }
        records
            .changes
            .retain(|id, _| records.tasks.contains_key(id));
    }
}

fn merge_effects(previous: TaskEffects, next: TaskEffects) -> TaskEffects {
    match (previous, next) {
        (TaskEffects::Unknown, _) | (_, TaskEffects::Unknown) => TaskEffects::Unknown,
        (TaskEffects::Partial, _) | (_, TaskEffects::Partial) => TaskEffects::Partial,
        (TaskEffects::Applied, _) | (_, TaskEffects::Applied) => TaskEffects::Applied,
        _ => TaskEffects::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };

    fn runner(config: TaskConfig) -> (TaskRunner, Arc<AtomicU64>) {
        let clock = Arc::new(AtomicU64::new(100));
        let source = clock.clone();
        let runner = TaskRunner::new(
            7,
            config,
            Box::new(move || {
                let value = source.load(Ordering::Relaxed);
                TaskTime {
                    unix_ms: 1000 + value,
                    monotonic_ms: value,
                }
            }),
        );
        (runner, clock)
    }

    fn child(runner: &TaskRunner, parent: TaskId, owner: &str) -> TaskId {
        let mut spec = TaskSpec::new("child", owner);
        spec.parent_id = Some(parent);
        runner.admit(spec).unwrap()
    }

    #[test]
    fn elapsed_time_includes_children_and_freezes_despite_wall_clock_changes() {
        let monotonic = Arc::new(AtomicU64::new(100));
        let wall = Arc::new(AtomicU64::new(10_000));
        let source = monotonic.clone();
        let wall_source = wall.clone();
        let runner = TaskRunner::new(
            7,
            TaskConfig::default(),
            Box::new(move || TaskTime {
                unix_ms: wall_source.load(Ordering::Relaxed),
                monotonic_ms: source.load(Ordering::Relaxed),
            }),
        );
        let parent = runner.admit(TaskSpec::new("script", "fixture")).unwrap();
        let child = child(&runner, parent, "fixture");
        monotonic.store(600, Ordering::Relaxed);
        runner.finish(parent, TaskOutcome::success(None, TaskEffects::None));
        assert_eq!(runner.elapsed_ms(parent), Ok(500));
        wall.store(1, Ordering::Relaxed);
        monotonic.store(2100, Ordering::Relaxed);
        runner.finish(child, TaskOutcome::success(None, TaskEffects::None));
        monotonic.store(5100, Ordering::Relaxed);
        assert_eq!(runner.elapsed_ms(parent), Ok(2000));
        assert_eq!(runner.elapsed_ms(child), Ok(2000));
    }

    fn success() -> TaskOutcome {
        TaskOutcome::success(None, TaskEffects::None)
    }

    #[test]
    fn executor_loss_cancels_children_of_uncancellable_body() {
        let (runner, _) = runner(TaskConfig::default());
        let mut spec = TaskSpec::new("script", "lost");
        spec.cancellable = false;
        let parent = runner.admit(spec).unwrap();
        let child = child(&runner, parent, "other");
        assert_eq!(runner.fail_owner("lost"), 1);
        assert!(runner.get(child).unwrap().cancel_requested);
        assert!(!runner.get(parent).unwrap().state.is_terminal());
        runner.finish(child, success());
        let snapshot = runner.get(parent).unwrap();
        assert!(
            matches!(snapshot.outcome.unwrap().status, TaskOutcomeStatus::Failure { error } if error.code == "executor_lost")
        );
    }

    #[test]
    fn list_filters_default_when_only_pagination_is_supplied() {
        let empty: TaskListRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, TaskListRequest::default());
        let page: TaskListRequest = serde_json::from_str(r#"{"limit":3}"#).unwrap();
        assert_eq!(page.limit, Some(3));
        assert!(!page.active_only);
    }

    #[test]
    fn admission_rejection_never_issues_identity() {
        let (runner, _) = runner(TaskConfig {
            max_active: 1,
            ..TaskConfig::default()
        });
        let first = runner.admit(TaskSpec::new("fetch", "native")).unwrap();
        assert_eq!(
            runner.admit(TaskSpec::new("fetch", "native")),
            Err(TaskStartError::Busy)
        );
        runner.finish(first, success());
        let next = runner.admit(TaskSpec::new("fetch", "native")).unwrap();
        assert_eq!(next.sequence(), first.sequence() + 1);
        assert_eq!(
            runner.get(TaskId::new(8, 1)),
            Err(TaskLookupError::WrongInstance)
        );
        assert_eq!(
            runner.get(TaskId::new(7, 0)),
            Err(TaskLookupError::NotFound)
        );
        assert_eq!(
            runner.get(TaskId::new(7, 20)),
            Err(TaskLookupError::NotFound)
        );
        assert_eq!(next.to_string().parse::<TaskId>().unwrap(), next);
    }

    #[test]
    fn parent_rejection_and_executor_ownership_preserve_state() {
        let (runner, _) = runner(TaskConfig::default());
        let mut spec = TaskSpec::new("script", "python");
        spec.parent_id = Some(TaskId::new(7, 500));
        assert_eq!(runner.admit(spec), Err(TaskStartError::InvalidParent));
        let id = runner.admit(TaskSpec::new("script", "python")).unwrap();
        assert_eq!(id.sequence(), 1);
        let before = runner.get(id).unwrap();
        assert_eq!(
            runner.started(id, "ipc").unwrap_err().code,
            "wrong_executor"
        );
        assert_eq!(
            runner.finish_owned(id, "ipc", success()).unwrap_err().code,
            "wrong_executor"
        );
        assert_eq!(runner.get(id).unwrap(), before);
        runner.finish(id, success());
        let mut spec = TaskSpec::new("child", "python");
        spec.parent_id = Some(id);
        assert_eq!(runner.admit(spec), Err(TaskStartError::InvalidParent));
    }

    #[test]
    fn completion_is_immutable_and_applying_rejects_duplicate_results() {
        let (runner, clock) = runner(TaskConfig::default());
        let id = runner.admit(TaskSpec::new("fetch", "native")).unwrap();
        assert!(runner.started(id, "native").unwrap());
        assert!(runner.begin_apply(id, 0));
        assert!(!runner.begin_apply(id, 0));
        assert_eq!(runner.cancel(id), Ok(TaskCancelReply::TooLate));
        assert_eq!(runner.get(id).unwrap().state, TaskState::Applying);
        clock.store(200, Ordering::Relaxed);
        runner.finish(id, success());
        let completed = runner.get(id).unwrap();
        assert_eq!(completed.state, TaskState::Succeeded);
        assert_eq!(completed.updated_at_ms, 1200);
        assert!(!runner.finish(id, TaskOutcome::failure("late", "duplicate")));
        assert!(!runner.started(id, "native").unwrap());
        assert!(!runner
            .output(
                id,
                "native",
                TaskDiagnostic {
                    level: "info".into(),
                    message: "late".into()
                }
            )
            .unwrap());
        assert_eq!(runner.get(id).unwrap(), completed);
        let changes = runner.take_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].revision, completed.revision);
        assert!(runner.take_changes().is_empty());
    }

    #[test]
    fn cancellation_waits_for_worker_acknowledgement_and_retains_effects() {
        let (runner, _) = runner(TaskConfig::default());
        let id = runner.admit(TaskSpec::new("script", "python")).unwrap();
        runner.started(id, "python").unwrap();
        runner
            .record_effects(id, "python", TaskEffects::Applied)
            .unwrap();
        assert_eq!(runner.cancel(id), Ok(TaskCancelReply::Requested));
        assert_eq!(runner.cancel(id), Ok(TaskCancelReply::AlreadyRequested));
        assert_eq!(runner.get(id).unwrap().state, TaskState::Running);
        assert_eq!(
            runner.can_apply_effect(id, "python", 0).unwrap_err().code,
            "cancelled"
        );
        assert_eq!(runner.cancellation_targets(), vec![(id, "python".into())]);
        assert!(!runner.begin_apply(id, 0));
        let cancelled = runner.get(id).unwrap();
        assert_eq!(cancelled.state, TaskState::Cancelled);
        assert_eq!(cancelled.effects, TaskEffects::Partial);
        assert!(runner.cancellation_targets().is_empty());
    }

    #[test]
    fn stale_context_rejects_effects_without_early_worker_completion() {
        let (runner, _) = runner(TaskConfig::default());
        let mut spec = TaskSpec::new("fetch", "native");
        spec.scene_epoch = Some(1);
        let id = runner.admit(spec).unwrap();
        assert_eq!(
            runner.can_apply_effect(id, "native", 2).unwrap_err().code,
            "stale_context"
        );
        assert!(!runner.get(id).unwrap().state.is_terminal());
        assert!(!runner.begin_apply(id, 2));
        let TaskOutcomeStatus::Failure { error } = runner.get(id).unwrap().outcome.unwrap().status
        else {
            panic!("expected stale failure")
        };
        assert_eq!(error.code, "stale_context");
    }

    #[test]
    fn parent_waits_for_children_and_keeps_evicted_failure() {
        let (runner, clock) = runner(TaskConfig {
            max_terminal: 2,
            terminal_ttl_ms: 10,
            ..TaskConfig::default()
        });
        let parent = runner.admit(TaskSpec::new("script", "python")).unwrap();
        let failed = child(&runner, parent, "native");
        let pending = child(&runner, parent, "native");
        runner.finish(failed, TaskOutcome::failure("parse_error", "bad structure"));
        clock.store(111, Ordering::Relaxed);
        assert_eq!(runner.get(failed), Err(TaskLookupError::Expired));
        runner.finish(parent, success());
        let waiting = runner.get(parent).unwrap();
        assert_eq!(waiting.state, TaskState::Running);
        assert_eq!(waiting.progress.unwrap().phase, "waiting_children");
        assert_eq!(runner.pending_count(), 2);
        assert!(!runner.finish(parent, success()));
        runner.finish(pending, TaskOutcome::success(None, TaskEffects::Applied));
        assert_eq!(runner.pending_count(), 0);
        let parent = runner.get(parent).unwrap();
        assert_eq!(parent.state, TaskState::Failed);
        assert_eq!(parent.effects, TaskEffects::Partial);
        assert!(parent
            .outcome
            .unwrap()
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("parse_error")));
    }

    #[test]
    fn parent_can_recover_child_failure_without_losing_history_or_waiting() {
        let (runner, _) = runner(TaskConfig::default());
        let mut spec = TaskSpec::new("agent", "plugin");
        spec.child_failure_policy = ChildFailurePolicy::ParentDecides;
        let parent = runner.admit(spec).unwrap();
        let failed = child(&runner, parent, "native");
        runner.finish(failed, TaskOutcome::failure("parse_error", "bad syntax"));
        let retry = child(&runner, parent, "native");
        runner.finish(parent, success());
        assert_eq!(runner.get(parent).unwrap().state, TaskState::Running);
        runner.finish(retry, TaskOutcome::success(None, TaskEffects::Applied));
        let result = runner.get(parent).unwrap();
        assert_eq!(result.state, TaskState::Succeeded);
        assert_eq!(result.effects, TaskEffects::Applied);
        assert_eq!(runner.get(failed).unwrap().state, TaskState::Failed);
        assert_eq!(runner.children_status(parent), Ok((0, 1)));
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("parse_error: bad syntax")));
    }

    #[test]
    fn parent_recovery_policy_cannot_override_cancellation_or_its_own_failure() {
        for cancel in [false, true] {
            let (runner, _) = runner(TaskConfig::default());
            let mut spec = TaskSpec::new("agent", "plugin");
            spec.child_failure_policy = ChildFailurePolicy::ParentDecides;
            let parent = runner.admit(spec).unwrap();
            let pending = child(&runner, parent, "native");
            if cancel {
                runner.cancel(parent).unwrap();
                assert!(runner.get(pending).unwrap().cancel_requested);
                runner.finish(parent, success());
            } else {
                runner.finish(parent, TaskOutcome::failure("step_limit", "no recovery"));
            }
            assert!(!runner.get(parent).unwrap().state.is_terminal());
            runner.finish(pending, success());
            assert_eq!(
                runner.get(parent).unwrap().state,
                if cancel {
                    TaskState::Cancelled
                } else {
                    TaskState::Failed
                }
            );
        }
    }

    #[test]
    fn silent_policy_is_inherited_without_discarding_output() {
        let (runner, _) = runner(TaskConfig::default());
        let mut spec = TaskSpec::new("agent", "plugin");
        spec.silent = true;
        let parent = runner.admit(spec).unwrap();
        let child = child(&runner, parent, "python");
        assert_eq!(runner.is_silent(child), Ok(true));
        runner
            .output(
                child,
                "python",
                TaskDiagnostic {
                    level: "info".into(),
                    message: "retained stdout".into(),
                },
            )
            .unwrap();
        runner.finish(child, success());
        assert_eq!(
            runner.get(child).unwrap().outcome.unwrap().diagnostics[0].message,
            "retained stdout"
        );
        let visible = runner.admit(TaskSpec::new("python", "python")).unwrap();
        assert_eq!(runner.is_silent(visible), Ok(false));
    }

    #[test]
    fn child_failure_preserves_original_detail_after_history_expires() {
        let (runner, clock) = runner(TaskConfig {
            terminal_ttl_ms: 10,
            ..TaskConfig::default()
        });
        let parent = runner.admit(TaskSpec::new("script", "python")).unwrap();
        let failed = child(&runner, parent, "native");
        runner.finish(failed, TaskOutcome::failure("parse_error", "bad structure"));
        clock.store(111, Ordering::Relaxed);
        assert_eq!(runner.get(failed), Err(TaskLookupError::Expired));
        runner.finish(parent, success());
        let outcome = runner.get(parent).unwrap().outcome.unwrap();
        assert!(
            matches!(outcome.status, TaskOutcomeStatus::Failure { error } if error.code == "child_failed")
        );
        assert!(outcome
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("parse_error: bad structure")));
    }

    #[test]
    fn cancelling_tree_waits_for_uncancellable_children_and_blocks_new_work() {
        let (runner, _) = runner(TaskConfig::default());
        let parent = runner.admit(TaskSpec::new("script", "python")).unwrap();
        let child_id = child(&runner, parent, "native");
        let mut spec = TaskSpec::new("uncancellable", "native");
        spec.cancellable = false;
        spec.parent_id = Some(parent);
        let uncancellable = runner.admit(spec).unwrap();
        runner.finish(parent, success());
        assert_eq!(runner.cancel(parent), Ok(TaskCancelReply::Requested));
        assert!(runner.get(child_id).unwrap().cancel_requested);
        assert!(!runner.get(uncancellable).unwrap().cancel_requested);
        assert_eq!(
            runner.cancel(uncancellable),
            Ok(TaskCancelReply::Unsupported)
        );
        let mut spec = TaskSpec::new("too_late", "native");
        spec.parent_id = Some(parent);
        assert_eq!(runner.admit(spec), Err(TaskStartError::InvalidParent));
        runner.finish(child_id, TaskOutcome::cancelled("requested"));
        assert_eq!(runner.get(parent).unwrap().state, TaskState::Running);
        runner.finish(
            uncancellable,
            TaskOutcome::success(None, TaskEffects::Applied),
        );
        let parent = runner.get(parent).unwrap();
        assert_eq!(parent.state, TaskState::Cancelled);
        assert_eq!(parent.effects, TaskEffects::Partial);
    }

    #[test]
    fn nested_children_complete_upward_and_lost_owners_cannot_leave_orphans() {
        let (runner, _) = runner(TaskConfig::default());
        let parent = runner.admit(TaskSpec::new("script", "python")).unwrap();
        let middle = child(&runner, parent, "python");
        let leaf = child(&runner, middle, "native");
        runner.finish(parent, success());
        assert_eq!(runner.fail_owner("python"), 1);
        assert!(runner.get(leaf).unwrap().cancel_requested);
        assert!(!runner.get(parent).unwrap().state.is_terminal());
        runner.finish(leaf, TaskOutcome::cancelled("requested"));
        let middle = runner.get(middle).unwrap();
        assert_eq!(middle.effects, TaskEffects::Unknown);
        assert_eq!(middle.state, TaskState::Failed);
        assert_eq!(runner.get(parent).unwrap().state, TaskState::Failed);
        assert_eq!(runner.pending_count(), 0);
        assert_eq!(runner.fail_owner("python"), 0);
    }

    #[test]
    fn waits_reject_ancestors_and_work_blocked_by_sequential_executor() {
        let (runner, _) = runner(TaskConfig::default());
        let parent = runner.admit(TaskSpec::new("script", "python")).unwrap();
        let network = child(&runner, parent, "native");
        let queued_python = child(&runner, network, "python");
        assert_eq!(
            runner
                .validate_wait(Some(parent), parent, Some("python"))
                .unwrap_err()
                .code,
            "would_deadlock"
        );
        assert_eq!(
            runner
                .validate_wait(Some(network), parent, None)
                .unwrap_err()
                .code,
            "would_deadlock"
        );
        assert_eq!(
            runner
                .validate_wait(Some(parent), network, Some("python"))
                .unwrap_err()
                .code,
            "would_deadlock"
        );
        assert!(runner.validate_wait(None, queued_python, None).is_ok());
        runner.finish(queued_python, success());
        assert!(runner
            .validate_wait(Some(parent), network, Some("python"))
            .is_ok());
        assert!(runner
            .validate_wait(Some(parent), queued_python, Some("python"))
            .is_ok());
    }

    #[test]
    fn history_uses_monotonic_time_and_expired_cursors_continue() {
        let (runner, clock) = runner(TaskConfig {
            max_terminal: 1,
            terminal_ttl_ms: 10,
            ..TaskConfig::default()
        });
        let first = runner.admit(TaskSpec::new("fetch", "native")).unwrap();
        runner.finish(first, success());
        clock.store(101, Ordering::Relaxed);
        let second = runner.admit(TaskSpec::new("fetch", "native")).unwrap();
        runner.finish(second, success());
        assert_eq!(runner.get(first), Err(TaskLookupError::Expired));
        let page = runner
            .list(&TaskListRequest {
                after: Some(first),
                ..TaskListRequest::default()
            })
            .unwrap();
        assert_eq!(page.tasks.len(), 1);
        assert_eq!(page.tasks[0].id, second);
        let active = runner.admit(TaskSpec::new("slow", "native")).unwrap();
        clock.store(111, Ordering::Relaxed);
        assert_eq!(runner.get(second), Err(TaskLookupError::Expired));
        assert!(runner.get(active).is_ok());
    }

    #[test]
    fn output_is_bounded_and_retained_in_terminal_outcome() {
        let (runner, _) = runner(TaskConfig::default());
        let mut spec = TaskSpec::new("\0".repeat(2000), "native");
        spec.message = "\0".repeat(2000);
        spec.origin = "\0".repeat(2000);
        let id = runner.admit(spec).unwrap();
        for _ in 0..20 {
            runner
                .output(
                    id,
                    "native",
                    TaskDiagnostic {
                        level: "\0".repeat(100),
                        message: "\0".repeat(1000),
                    },
                )
                .unwrap();
        }
        let running = runner.get(id).unwrap();
        assert!(running.diagnostics_truncated);
        assert!(!running.diagnostics.is_empty());
        assert!(running.diagnostics.len() <= MAX_DIAGNOSTICS);
        assert!(serde_json::to_vec(&running).unwrap().len() <= MAX_TASK_SNAPSHOT_BYTES);
        runner.finish(id, TaskOutcome::failure("parse_error", "bad input"));
        let finished = runner.get(id).unwrap();
        assert!(serde_json::to_vec(&finished).unwrap().len() <= MAX_TASK_SNAPSHOT_BYTES);
        let outcome = finished.outcome.unwrap();
        assert_eq!(outcome.diagnostics, running.diagnostics);
        assert!(
            matches!(outcome.status, TaskOutcomeStatus::Failure { error } if error.code == "parse_error")
        );
        assert!(outcome.diagnostics_truncated);
    }

    #[test]
    fn oversized_producer_result_preserves_effects_as_failure() {
        let (runner, _) = runner(TaskConfig::default());
        let id = runner.admit(TaskSpec::new("fetch", "native")).unwrap();
        let outcome = TaskOutcome::success(
            Some(TaskData {
                kind: "huge".into(),
                schema_version: 1,
                payload: serde_json::Value::String("x".repeat(MAX_TASK_SNAPSHOT_BYTES)),
            }),
            TaskEffects::Applied,
        );
        assert_eq!(
            runner.config().validate_outcome(&outcome).unwrap_err().code,
            "result_too_large"
        );
        runner.finish(id, outcome);
        let outcome = runner.get(id).unwrap().outcome.unwrap();
        assert!(
            matches!(outcome.status, TaskOutcomeStatus::Failure { error } if error.code == "result_too_large")
        );
        assert_eq!(outcome.effects, TaskEffects::Partial);
    }
}
