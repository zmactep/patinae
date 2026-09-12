//! Native execution adapter for the portable task lifecycle.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::hash::BuildHasher;
use std::pin::Pin;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use patinae_cmd::tasks::{
    TaskConfig, TaskError, TaskOutcome, TaskProgress, TaskSpec, TaskStartError, TaskTime,
};
pub use patinae_cmd::tasks::{TaskId, TaskRunner};
use tokio::{runtime::Runtime, task::AbortHandle};

use crate::kernel::AppKernel;

const NATIVE_OWNER: &str = "native";

/// Create one native session's authoritative task records and clock.
pub fn native_task_runner() -> TaskRunner {
    let instance = ((std::collections::hash_map::RandomState::new().hash_one(0_u8) as u128) << 64)
        | std::collections::hash_map::RandomState::new().hash_one(1_u8) as u128;
    let epoch = Instant::now();
    TaskRunner::new(
        instance,
        TaskConfig::default(),
        Box::new(move || TaskTime {
            unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            monotonic_ms: epoch.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        }),
    )
}

/// Executable result applied by the owning host before completion.
pub trait TaskResult: Send + 'static {
    /// Validate constraints before changing host state.
    fn preflight(&self, _kernel: &AppKernel) -> Result<(), TaskError> {
        Ok(())
    }
    /// Apply mandatory host effects and return the retained outcome.
    fn apply(self: Box<Self>, kernel: &mut AppKernel, id: TaskId) -> TaskOutcome;
}

/// Constructs an executable failure result after a worker failure.
pub type TaskFailureHandler = Box<dyn FnOnce(String) -> Box<dyn TaskResult> + Send>;

/// Background computation executed by the native adapter.
pub trait AsyncTask: Send + 'static {
    fn notification_message(&self) -> String;
    fn kind(&self) -> &str {
        "background"
    }
    fn origin(&self) -> String {
        "host".into()
    }
    fn cancellable(&self) -> bool {
        true
    }
    fn scene_epoch(&self) -> Option<u64> {
        None
    }
    fn failure_handler(&self) -> TaskFailureHandler {
        Box::new(|error| Box::new(TaskFailureResult(error)))
    }
    fn execute(self: Box<Self>) -> Pin<Box<dyn Future<Output = Box<dyn TaskResult>> + Send>>;
}

struct TaskFailureResult(String);
impl TaskResult for TaskFailureResult {
    fn apply(self: Box<Self>, _kernel: &mut AppKernel, _id: TaskId) -> TaskOutcome {
        TaskOutcome::failure("worker_failed", self.0)
    }
}

/// Owned completion envelope; receiving it does not mark a task complete.
pub struct ReadyTask {
    pub id: TaskId,
    pub result: Box<dyn TaskResult>,
    pub(crate) runtime_failure: Option<String>,
}

enum WorkerMessage {
    Started(TaskId),
    Ready(ReadyTask),
}

/// Native worker handles and transport, without task lifecycle records.
pub struct NativeTaskExecutor {
    runtime: Runtime,
    sender: Sender<WorkerMessage>,
    receiver: Receiver<WorkerMessage>,
    workers: RefCell<BTreeMap<TaskId, AbortHandle>>,
    ready: RefCell<VecDeque<ReadyTask>>,
}

impl Default for NativeTaskExecutor {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            runtime: Runtime::new().expect("failed to create task runtime"),
            sender,
            receiver,
            workers: RefCell::new(BTreeMap::new()),
            ready: RefCell::new(VecDeque::new()),
        }
    }
}

impl NativeTaskExecutor {
    /// Admit work before starting its worker.
    pub fn spawn(
        &self,
        tasks: &TaskRunner,
        task: Box<dyn AsyncTask>,
        parent_id: Option<TaskId>,
    ) -> Result<TaskId, TaskStartError> {
        self.spawn_with_silent(tasks, task, parent_id, false)
    }

    pub(crate) fn spawn_with_silent(
        &self,
        tasks: &TaskRunner,
        task: Box<dyn AsyncTask>,
        parent_id: Option<TaskId>,
        silent: bool,
    ) -> Result<TaskId, TaskStartError> {
        let spec = TaskSpec {
            kind: task.kind().into(),
            origin: task.origin(),
            owner: NATIVE_OWNER.into(),
            parent_id,
            scene_epoch: task.scene_epoch(),
            cancellable: task.cancellable(),
            child_failure_policy: Default::default(),
            silent,
            message: task.notification_message(),
        };
        let id = tasks.admit(spec)?;
        let failure_handler = task.failure_handler();
        let sender = self.sender.clone();
        let started = sender.clone();
        let worker = self.runtime.spawn(async move {
            let _ = started.send(WorkerMessage::Started(id));
            task.execute().await
        });
        self.workers.borrow_mut().insert(id, worker.abort_handle());
        self.runtime.spawn(async move {
            let (result, runtime_failure) = match worker.await {
                Ok(result) => (result, None),
                Err(error) => {
                    let detail = if error.is_panic() {
                        "task panicked"
                    } else {
                        "task was cancelled"
                    }
                    .to_owned();
                    (failure_handler(detail.clone()), Some(detail))
                }
            };
            let _ = sender.send(WorkerMessage::Ready(ReadyTask {
                id,
                result,
                runtime_failure,
            }));
        });
        Ok(id)
    }

    /// Observe cancellation and worker events without waiting for computation.
    pub fn poll(&self, tasks: &TaskRunner) -> Option<ReadyTask> {
        for (id, owner) in tasks.cancellation_targets() {
            if owner == NATIVE_OWNER {
                if let Some(worker) = self.workers.borrow().get(&id) {
                    worker.abort();
                }
            }
        }
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                WorkerMessage::Started(id) => {
                    let _ = tasks.started(id, NATIVE_OWNER);
                }
                WorkerMessage::Ready(ready) => {
                    if self.workers.borrow_mut().remove(&ready.id).is_none() {
                        continue;
                    }
                    if let Ok(snapshot) = tasks.get(ready.id) {
                        if snapshot.state.is_terminal() {
                            continue;
                        }
                        let _ = tasks.progress(
                            ready.id,
                            NATIVE_OWNER,
                            TaskProgress {
                                phase: "awaiting_apply".into(),
                                message: snapshot.progress.map_or_else(String::new, |p| p.message),
                                completed: None,
                                total: None,
                            },
                        );
                        self.ready.borrow_mut().push_back(ready);
                    }
                }
            }
        }
        self.ready.borrow_mut().pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_cmd::tasks::{TaskEffects, TaskOutcomeStatus, TaskState};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    struct GatedTask {
        gate: tokio::sync::oneshot::Receiver<()>,
        effects: Arc<AtomicUsize>,
        panic: bool,
    }
    struct CountResult(Arc<AtomicUsize>);
    impl TaskResult for CountResult {
        fn apply(self: Box<Self>, kernel: &mut AppKernel, id: TaskId) -> TaskOutcome {
            assert_eq!(kernel.tasks.get(id).unwrap().state, TaskState::Applying);
            self.0.fetch_add(1, Ordering::SeqCst);
            TaskOutcome::success(None, TaskEffects::Applied)
        }
    }
    impl AsyncTask for GatedTask {
        fn notification_message(&self) -> String {
            "Waiting for test gate".into()
        }
        fn execute(self: Box<Self>) -> Pin<Box<dyn Future<Output = Box<dyn TaskResult>> + Send>> {
            Box::pin(async move {
                let _ = self.gate.await;
                assert!(!self.panic, "test worker panic");
                Box::new(CountResult(self.effects)) as Box<dyn TaskResult>
            })
        }
    }
    fn drive(kernel: &mut AppKernel, id: TaskId) -> patinae_cmd::tasks::TaskSnapshot {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            kernel.process_async_tasks(None, (1, 1));
            let snapshot = kernel.tasks.get(id).unwrap();
            if snapshot.state.is_terminal() {
                return snapshot;
            }
            assert!(Instant::now() < deadline, "native worker did not complete");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn completion_is_published_only_after_host_application() {
        let mut kernel = AppKernel::new();
        let effects = Arc::new(AtomicUsize::new(0));
        let (send, gate) = tokio::sync::oneshot::channel();
        let id = kernel
            .spawn_task(GatedTask {
                gate,
                effects: effects.clone(),
                panic: false,
            })
            .unwrap();
        assert!(kernel.tasks.get(id).unwrap().outcome.is_none());
        send.send(()).unwrap();
        let done = drive(&mut kernel, id);
        assert_eq!(done.state, TaskState::Succeeded);
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(kernel.tasks.get(id).unwrap(), done);
    }
    #[test]
    fn cancellation_waits_for_observer_and_prevents_application() {
        let mut kernel = AppKernel::new();
        let effects = Arc::new(AtomicUsize::new(0));
        let (_send, gate) = tokio::sync::oneshot::channel();
        let id = kernel
            .spawn_task(GatedTask {
                gate,
                effects: effects.clone(),
                panic: false,
            })
            .unwrap();
        kernel.tasks.cancel(id).unwrap();
        assert!(!kernel.tasks.get(id).unwrap().state.is_terminal());
        assert!(!kernel
            .output
            .buffer
            .iter()
            .any(|m| m.text.starts_with("Cancelled:")));
        assert_eq!(drive(&mut kernel, id).state, TaskState::Cancelled);
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        let cancellation = format!("Cancelled: task {id}");
        assert_eq!(
            kernel
                .output
                .buffer
                .iter()
                .filter(|m| m.text == cancellation)
                .count(),
            1
        );
        let output_len = kernel.output.buffer.len();
        kernel.process_async_tasks(None, (1, 1));
        assert_eq!(kernel.output.buffer.len(), output_len);
    }
    #[test]
    fn worker_panic_is_retained_as_failure() {
        let mut kernel = AppKernel::new();
        let (send, gate) = tokio::sync::oneshot::channel();
        let id = kernel
            .spawn_task(GatedTask {
                gate,
                effects: Arc::new(AtomicUsize::new(0)),
                panic: true,
            })
            .unwrap();
        send.send(()).unwrap();
        let done = drive(&mut kernel, id);
        assert!(
            matches!(done.outcome.unwrap().status, TaskOutcomeStatus::Failure { error } if error.code == "worker_failed")
        );
    }
}
