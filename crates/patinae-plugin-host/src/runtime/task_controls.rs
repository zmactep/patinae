//! Plugin-owned task requests, acknowledgements and pending waits.

use super::*;

pub(super) fn checked_task_owner(
    tasks: &patinae_cmd::tasks::TaskRunner,
    id: patinae_cmd::tasks::TaskId,
    plugin: &str,
) -> Option<String> {
    let owner = tasks.owner(id).ok()?;
    (owner == plugin || owner.starts_with(&format!("{plugin}/"))).then_some(owner)
}

pub(crate) struct PendingTaskWait {
    plugin_index: usize,
    id: u64,
    task_id: patinae_cmd::tasks::TaskId,
    deadline: Option<std::time::Instant>,
    waiter: Option<patinae_cmd::tasks::TaskId>,
    serial_owner: Option<String>,
}

// Bound observer state without limiting task lifetime.
const MAX_PENDING_TASK_WAITS: usize = 1024;

impl PluginHost {
    /// Apply producer messages after shared reads without blocking the host pump.
    pub fn apply_task_controls<'a>(
        &mut self,
        kernel: &mut patinae_framework::kernel::AppKernel,
        mut render_context: Option<&'a mut (dyn patinae_scene::CaptureRenderer + 'a)>,
        viewport_size: (u32, u32),
    ) {
        self.host_query_results
            .resize_with(self.plugins.len(), Vec::new);
        for plugin in &self.plugins {
            if plugin.faulted {
                let owners: std::collections::HashSet<_> = kernel
                    .tasks
                    .active_snapshots()
                    .iter()
                    .filter_map(|task| {
                        checked_task_owner(&kernel.tasks, task.id, &plugin.metadata.name)
                    })
                    .collect();
                for owner in owners {
                    kernel.tasks.fail_owner(&owner);
                }
            }
        }
        for (plugin_index, query) in std::mem::take(&mut self.pending_task_controls) {
            let Some(plugin) = self.plugins.get(plugin_index) else {
                continue;
            };
            let owner = plugin.metadata.name.clone();
            let (id, value) = match query {
                WireHostQuery::ForgetTaskWait { id } => {
                    self.pending_task_waits
                        .retain(|wait| wait.plugin_index != plugin_index || wait.id != id);
                    continue;
                }
                WireHostQuery::FailTaskOwner { id, owner_tag } => {
                    let count = kernel.tasks.fail_owner(&format!("{owner}/{owner_tag}"));
                    (id, WireHostQueryValue::TaskAcknowledged(Ok(count > 0)))
                }

                WireHostQuery::CancelTask { id, task_id } => (
                    id,
                    WireHostQueryValue::TaskCancel(kernel.tasks.cancel(task_id)),
                ),
                WireHostQuery::StartTask {
                    id,
                    mut request,
                    parent_id,
                } => {
                    request.executor = owner.clone();
                    let valid_parent = parent_id.is_none_or(|parent| {
                        checked_task_owner(&kernel.tasks, parent, &owner).is_some()
                    });
                    let result = if plugin.faulted {
                        Err(patinae_cmd::tasks::TaskStartError::ExecutorUnavailable)
                    } else if !valid_parent {
                        Err(patinae_cmd::tasks::TaskStartError::InvalidParent)
                    } else {
                        kernel.start_plugin_task(request, parent_id)
                    };
                    (id, WireHostQueryValue::TaskStarted(result))
                }
                WireHostQuery::TaskEvent { id, task_id, event } => {
                    let owner = checked_task_owner(&kernel.tasks, task_id, &owner).unwrap_or(owner);
                    use patinae_cmd::tasks::TaskEvent;
                    let result = match event {
                        TaskEvent::Started => kernel.tasks.started(task_id, &owner),
                        TaskEvent::Progress(progress) => {
                            log::info!(
                                "Plugin task [plugin={}, task={}]: {}",
                                owner,
                                task_id,
                                progress.message
                            );
                            kernel.tasks.progress(task_id, &owner, progress)
                        }
                        TaskEvent::Output(output) => {
                            let accepted = kernel.tasks.output(task_id, &owner, output.clone());
                            if matches!(accepted, Ok(true)) {
                                kernel.present_task_output(task_id, &output);
                            }
                            accepted
                        }
                        TaskEvent::Finished(outcome) => {
                            kernel.tasks.finish_owned(task_id, &owner, outcome)
                        }
                    };
                    (id, WireHostQueryValue::TaskAcknowledged(result))
                }
                WireHostQuery::TaskCommand {
                    id,
                    task_id,
                    command,
                    silent,
                } => {
                    let owner = checked_task_owner(&kernel.tasks, task_id, &owner).unwrap_or(owner);
                    log::info!(
                        "Plugin command [plugin={}, task={}, request={}]: {:?}",
                        owner,
                        task_id,
                        id,
                        command
                    );
                    let result = kernel
                        .execute_task_command(
                            task_id,
                            &owner,
                            &command,
                            silent,
                            match &mut render_context {
                                Some(renderer) => Some(&mut **renderer),
                                None => None,
                            },
                            viewport_size,
                        )
                        .map(patinae_cmd::CommandReply::from);
                    (id, WireHostQueryValue::TaskCommand(result))
                }
                WireHostQuery::TaskAction {
                    id,
                    task_id,
                    action,
                } => {
                    let owner = checked_task_owner(&kernel.tasks, task_id, &owner).unwrap_or(owner);
                    let result = kernel
                        .tasks
                        .can_apply_effect(task_id, &owner, kernel.session.task_epoch())
                        .and_then(|()| {
                            let effects = crate::actions::apply_task_action(kernel, action)?;
                            if effects.panel_update {
                                self.bump_panel_ui_generation();
                            }
                            kernel.tasks.record_effects(task_id, &owner, effects.scene)
                        });
                    (id, WireHostQueryValue::TaskAcknowledged(result))
                }
                WireHostQuery::WaitTask {
                    id,
                    task_id,
                    waiter,
                    timeout_ms,
                } => {
                    let check = if waiter.is_some_and(|waiter| {
                        checked_task_owner(&kernel.tasks, waiter, &owner).is_none()
                    }) {
                        Err(patinae_cmd::tasks::TaskError::new(
                            "wrong_executor",
                            "waiter belongs to another executor",
                        ))
                    } else {
                        kernel.tasks.validate_wait(
                            waiter,
                            task_id,
                            waiter
                                .and_then(|id| checked_task_owner(&kernel.tasks, id, &owner))
                                .as_deref(),
                        )
                    };
                    if let Err(error) = check {
                        (id, WireHostQueryValue::TaskWait(Err(error)))
                    } else {
                        if self.pending_task_waits.len() >= MAX_PENDING_TASK_WAITS {
                            (
                                id,
                                WireHostQueryValue::TaskWait(Err(
                                    patinae_cmd::tasks::TaskError::new(
                                        "busy",
                                        "too many pending waits",
                                    ),
                                )),
                            )
                        } else {
                            self.pending_task_waits.push(PendingTaskWait {
                                plugin_index,
                                id,
                                task_id,
                                waiter,
                                serial_owner: waiter
                                    .and_then(|id| checked_task_owner(&kernel.tasks, id, &owner)),
                                deadline: timeout_ms.and_then(|ms| {
                                    std::time::Instant::now()
                                        .checked_add(std::time::Duration::from_millis(ms))
                                }),
                            });
                            continue;
                        }
                    }
                }
                _ => continue,
            };
            self.host_query_results[plugin_index].push(WireHostQueryResult {
                id,
                result: Ok(value),
            });
        }
        let now = std::time::Instant::now();
        self.pending_task_waits.retain(|wait| {
            let result = match kernel.tasks.validate_wait(
                wait.waiter,
                wait.task_id,
                wait.serial_owner.as_deref(),
            ) {
                Err(error) => Some(Err(error)),
                Ok(()) => match kernel.tasks.get(wait.task_id) {
                    Ok(snapshot) if snapshot.state.is_terminal() => Some(Ok(snapshot)),
                    Err(error) => Some(Err(patinae_cmd::tasks::TaskError::new(
                        error.to_string(),
                        error.to_string(),
                    ))),
                    _ if wait.deadline.is_some_and(|deadline| now >= deadline) => Some(Err(
                        patinae_cmd::tasks::TaskError::new("timeout", "task wait timed out"),
                    )),
                    _ => None,
                },
            };
            if let Some(result) = result {
                if let Some(replies) = self.host_query_results.get_mut(wait.plugin_index) {
                    replies.push(WireHostQueryResult {
                        id: wait.id,
                        result: Ok(WireHostQueryValue::TaskWait(result)),
                    });
                }
                false
            } else {
                true
            }
        });
        self.prepare_task_dispatch(kernel);
    }
}

#[cfg(test)]
mod tasks {
    //! Task queries through static plugins and the production dynamic ABI adapters.

    use crate::host::tests::{host_with_message_handlers, test_declaration, SharedFixture};
    use crate::loader::load_declaration_for_test;
    use crate::PluginHost;
    use patinae_cmd::CommandExecutor;
    use patinae_framework::message::AppMessage;
    use patinae_framework::message::MessageBus;
    use patinae_plugin::ffi::AbiStatus;
    use patinae_plugin::ffi::HostCallbacks;
    use patinae_plugin::ffi::HostRegistrarHandle;
    use patinae_plugin::ffi::PluginRegisterFn;
    use patinae_plugin::ffi::CAPABILITY_MESSAGE_RUNTIME;
    use patinae_plugin::registrar::MessageHandler;
    use patinae_plugin::registrar::PluginMetadata;
    use patinae_plugin::registrar::PluginRegistrar;
    use patinae_plugin::registrar::PollContext;
    use patinae_plugin::tasks::{TaskCancelReply, TaskId, TaskListRequest, TaskLookupError};
    use patinae_plugin::wire;
    use patinae_plugin::wire::WireHostQuery;
    use patinae_plugin::wire::WireHostQueryResult;
    use patinae_plugin::wire::WireHostQueryValue;

    struct TaskRequester {
        name: &'static str,
        queries: Vec<WireHostQuery>,
    }

    impl MessageHandler for TaskRequester {
        fn on_message(&mut self, message: &AppMessage, _bus: &mut MessageBus) {
            if let AppMessage::Custom { topic, payload } = message {
                if topic == self.name {
                    self.queries
                        .extend(wire::decode::<Vec<WireHostQuery>>(payload).unwrap());
                }
            }
        }

        fn needs_poll(&self) -> bool {
            true
        }

        fn poll(&mut self, ctx: &mut PollContext<'_>) {
            for invocation in ctx.task_invocations {
                ctx.bus.send(AppMessage::Custom {
                    topic: format!("{}:invocation", self.name),
                    payload: wire::encode(&invocation.task_id).unwrap(),
                });
            }
            for query in self.queries.drain(..) {
                match query {
                    WireHostQuery::GetTask { id, task_id } => ctx.get_task(id, task_id),
                    WireHostQuery::ListTasks { id, request } => ctx.list_tasks(id, request),
                    WireHostQuery::CancelTask { id, task_id } => ctx.cancel_task(id, task_id),
                    _ => ctx.query_host(query),
                }
            }
            for reply in ctx.host_query_results {
                assert!(ctx.task_result(reply.id).is_some());
                ctx.bus.send(AppMessage::Custom {
                    topic: format!("{}:task-response", self.name),
                    payload: wire::encode(reply).unwrap(),
                });
            }
        }
    }

    unsafe fn register_task_requester(
        handle: HostRegistrarHandle,
        callbacks: *const HostCallbacks,
        name: &'static str,
    ) -> AbiStatus {
        // SAFETY: Registration arguments are forwarded unchanged from the host callback.
        let Ok(mut registrar) = (unsafe { PluginRegistrar::from_abi(handle, callbacks) }) else {
            return AbiStatus::INVALID;
        };
        registrar.set_metadata(PluginMetadata::new(name, "1.0", "Task query fixture"));
        registrar.set_message_handler(TaskRequester {
            name,
            queries: Vec::new(),
        });
        registrar.finish()
    }

    unsafe extern "C" fn register_first(
        handle: HostRegistrarHandle,
        callbacks: *const HostCallbacks,
    ) -> AbiStatus {
        // SAFETY: Inputs were supplied by the host for this registration call.
        unsafe { register_task_requester(handle, callbacks, "first") }
    }

    unsafe extern "C" fn register_second(
        handle: HostRegistrarHandle,
        callbacks: *const HostCallbacks,
    ) -> AbiStatus {
        // SAFETY: Inputs were supplied by the host for this registration call.
        unsafe { register_task_requester(handle, callbacks, "second") }
    }

    fn host(dynamic: bool) -> PluginHost {
        if !dynamic {
            return host_with_message_handlers(vec![
                Box::new(TaskRequester {
                    name: "first",
                    queries: Vec::new(),
                }),
                Box::new(TaskRequester {
                    name: "second",
                    queries: Vec::new(),
                }),
            ]);
        }
        let mut host = PluginHost::new();
        let mut executor = CommandExecutor::new();
        for register in [register_first as PluginRegisterFn, register_second] {
            let mut declaration = test_declaration(Some(register));
            declaration.capabilities |= CAPABILITY_MESSAGE_RUNTIME;
            load_declaration_for_test(&mut host, &mut executor, declaration).unwrap();
        }
        host
    }

    fn request(
        host: &mut PluginHost,
        bus: &mut MessageBus,
        name: &str,
        queries: Vec<WireHostQuery>,
    ) {
        host.broadcast(
            &AppMessage::Custom {
                topic: name.into(),
                payload: wire::encode(&queries).unwrap(),
            },
            bus,
        );
    }

    fn replies(bus: &mut MessageBus) -> Vec<(String, WireHostQueryResult)> {
        bus.drain_outbox()
            .into_iter()
            .filter_map(|message| {
                if let AppMessage::Custom { topic, payload } = message {
                    if topic.ends_with(":task-response") {
                        return Some((topic, wire::decode(&payload).unwrap()));
                    }
                }
                None
            })
            .collect()
    }

    fn check_owner_routing(dynamic: bool) {
        let mut host = host(dynamic);
        let tasks = patinae_framework::tasks::native_task_runner();
        let first = tasks
            .admit(patinae_cmd::tasks::TaskSpec::new("test", "test"))
            .unwrap();
        let second = tasks
            .admit(patinae_cmd::tasks::TaskSpec::new("test", "test"))
            .unwrap();
        let fixture = SharedFixture::new();
        let mut bus = MessageBus::new();
        request(
            &mut host,
            &mut bus,
            "first",
            vec![WireHostQuery::CancelTask {
                id: 7,
                task_id: first,
            }],
        );
        request(
            &mut host,
            &mut bus,
            "second",
            vec![WireHostQuery::CancelTask {
                id: 7,
                task_id: second,
            }],
        );
        {
            let mut shared = fixture.shared();
            shared.tasks = Some(&tasks);
            host.poll_all(&shared, &mut bus);
            assert!(!tasks.get(first).unwrap().cancel_requested);
            assert!(!tasks.get(second).unwrap().cancel_requested);
        }
        let mut kernel = patinae_framework::kernel::AppKernel::new();
        kernel.tasks = tasks;
        host.apply_task_controls(&mut kernel, None, (1, 1));
        let tasks = kernel.tasks;
        assert!(tasks.get(first).unwrap().cancel_requested);
        assert!(tasks.get(second).unwrap().cancel_requested);
        let mut shared = fixture.shared();
        shared.tasks = Some(&tasks);
        host.poll_all(&shared, &mut bus);
        let replies = replies(&mut bus);
        assert_eq!(replies.len(), 2);
        for ((topic, reply), expected) in replies
            .into_iter()
            .zip(["first:task-response", "second:task-response"])
        {
            assert_eq!(topic, expected);
            assert_eq!(reply.id, 7);
            assert!(matches!(
                reply.result,
                Ok(WireHostQueryValue::TaskCancel(Ok(
                    TaskCancelReply::Requested
                )))
            ));
        }
        assert!(
            host.command_owners.is_empty(),
            "task replies must not retain command routing tokens"
        );
    }

    #[derive(Default)]
    struct RecordingRenderer {
        captures: Vec<(std::path::PathBuf, u32, u32)>,
        fail: bool,
    }

    impl patinae_scene::CaptureRenderer for RecordingRenderer {
        fn gpu_device(&self) -> &std::sync::Arc<wgpu::Device> {
            panic!("PNG routing does not require a GPU device")
        }

        fn gpu_queue(&self) -> &std::sync::Arc<wgpu::Queue> {
            panic!("PNG routing does not require a GPU queue")
        }

        fn capture_png(
            &mut self,
            path: &std::path::Path,
            width: u32,
            height: u32,
            _camera: &mut patinae_scene::Camera,
            _registry: &mut patinae_scene::ObjectRegistry,
            _settings: &patinae_settings::Settings,
            _named: &patinae_scene::NamedPalette,
            _themed: &patinae_scene::ThemedPalette,
            _clear_color: [f32; 3],
        ) -> Result<(), patinae_scene::ViewerError> {
            self.captures.push((path.to_owned(), width, height));
            if self.fail {
                Err(patinae_scene::ViewerError::capture_error(
                    "capture fixture failed",
                ))
            } else {
                Ok(())
            }
        }
    }

    fn task_queries(
        host: &mut PluginHost,
        kernel: &mut patinae_framework::kernel::AppKernel,
        plugin: &str,
        queries: Vec<WireHostQuery>,
        renderer: Option<&mut dyn patinae_scene::CaptureRenderer>,
        viewport_size: (u32, u32),
    ) -> Vec<WireHostQueryResult> {
        let fixture = SharedFixture::new();
        request(host, &mut kernel.bus, plugin, queries);
        {
            let mut shared = fixture.shared();
            shared.tasks = Some(&kernel.tasks);
            host.poll_all(&shared, &mut kernel.bus);
        }
        host.apply_task_controls(kernel, renderer, viewport_size);
        let mut shared = fixture.shared();
        shared.tasks = Some(&kernel.tasks);
        host.poll_all(&shared, &mut kernel.bus);
        replies(&mut kernel.bus)
            .into_iter()
            .map(|(topic, reply)| {
                assert_eq!(topic, format!("{plugin}:task-response"));
                reply
            })
            .collect()
    }

    #[test]
    fn dynamic_task_png_preserves_renderer_viewport_errors_and_guards() {
        let mut host = host(true);
        let mut kernel = patinae_framework::kernel::AppKernel::new();
        kernel.executor.register_task_executor("first");
        let mut description =
            patinae_cmd::PluginTaskRequest::new("capture", serde_json::Value::Null);
        description.executor = "first".into();
        let task_id = kernel.start_plugin_task(description, None).unwrap();
        let command = |id, text: &str| WireHostQuery::TaskCommand {
            id,
            task_id,
            command: text.into(),
            silent: true,
        };
        let mut renderer = RecordingRenderer::default();
        let responses = task_queries(
            &mut host,
            &mut kernel,
            "first",
            vec![
                command(11, "png default.png"),
                command(12, "png explicit.png, 80, 40"),
            ],
            Some(&mut renderer),
            (320, 180),
        );
        assert_eq!(responses.len(), 2);
        for (response, id) in responses.iter().zip([11, 12]) {
            assert_eq!(response.id, id);
            let Ok(WireHostQueryValue::TaskCommand(Ok(reply))) = &response.result else {
                panic!("expected command receipt");
            };
            assert!(reply.result.is_ok(), "{:?}", reply.result);
            assert!(reply.task_ids.is_empty());
            assert!(reply
                .messages
                .iter()
                .any(|message| message.text.contains("Saved")));
        }
        assert_eq!(
            renderer.captures,
            vec![
                ("default.png".into(), 320, 180),
                ("explicit.png".into(), 80, 40)
            ]
        );

        renderer.fail = true;
        for (with_renderer, expected) in [
            (true, "capture fixture failed"),
            (false, "No render context available"),
        ] {
            let responses = task_queries(
                &mut host,
                &mut kernel,
                "first",
                vec![command(13, "png failed.png")],
                if with_renderer {
                    Some(&mut renderer)
                } else {
                    None
                },
                (320, 180),
            );
            let Ok(WireHostQueryValue::TaskCommand(Ok(reply))) = &responses[0].result else {
                panic!("expected command failure receipt");
            };
            assert!(reply.result.as_ref().unwrap_err().contains(expected));
        }
        assert_eq!(renderer.captures.len(), 3);

        for (plugin, expected) in [("second", "wrong_executor"), ("first", "cancelled")] {
            if plugin == "first" {
                kernel.tasks.cancel(task_id).unwrap();
            }
            let responses = task_queries(
                &mut host,
                &mut kernel,
                plugin,
                vec![command(14, "png forbidden.png")],
                Some(&mut renderer),
                (320, 180),
            );
            let Ok(WireHostQueryValue::TaskCommand(Err(error))) = &responses[0].result else {
                panic!("expected task guard failure");
            };
            assert_eq!(error.code, expected);
        }
        assert_eq!(
            renderer.captures.len(),
            3,
            "rejected tasks must not reach the renderer"
        );
    }

    #[test]
    fn dynamic_task_pml_child_captures_with_the_current_host_viewport() {
        use patinae_cmd::tasks::{TaskEffects, TaskOutcome, TaskState};
        let mut host = host(true);
        let mut kernel = patinae_framework::kernel::AppKernel::new();
        kernel.executor.register_task_executor("first");
        let mut description =
            patinae_cmd::PluginTaskRequest::new("script", serde_json::Value::Null);
        description.executor = "first".into();
        let parent = kernel.start_plugin_task(description, None).unwrap();
        let path = std::env::temp_dir().join(format!("patinae-task-capture-{parent}.pml"));
        std::fs::write(
            &path,
            "png script-default.png\npng script-explicit.png, 64, 32\n",
        )
        .unwrap();
        let responses = task_queries(
            &mut host,
            &mut kernel,
            "first",
            vec![WireHostQuery::TaskCommand {
                id: 21,
                task_id: parent,
                command: format!("run {}", serde_json::to_string(&path).unwrap()),
                silent: true,
            }],
            None,
            (1, 1),
        );
        let Ok(WireHostQueryValue::TaskCommand(Ok(reply))) = &responses[0].result else {
            panic!("expected script admission");
        };
        assert!(reply.result.is_ok(), "{:?}", reply.result);
        assert_eq!(reply.task_ids.len(), 1);
        let child = reply.task_ids[0];
        assert_eq!(kernel.tasks.get(child).unwrap().parent_id, Some(parent));
        kernel
            .tasks
            .finish_owned(
                parent,
                "first",
                TaskOutcome::success(None, TaskEffects::None),
            )
            .unwrap();
        assert!(!kernel.tasks.get(parent).unwrap().state.is_terminal());
        let mut renderer = RecordingRenderer::default();
        for _ in 0..8 {
            kernel.process_async_tasks(Some(&mut renderer), (640, 360));
            if kernel.tasks.get(parent).unwrap().state.is_terminal() {
                break;
            }
        }
        std::fs::remove_file(path).unwrap();
        assert_eq!(kernel.tasks.get(child).unwrap().state, TaskState::Succeeded);
        assert_eq!(
            kernel.tasks.get(parent).unwrap().state,
            TaskState::Succeeded
        );
        assert_eq!(
            renderer.captures,
            vec![
                ("script-default.png".into(), 640, 360),
                ("script-explicit.png".into(), 64, 32)
            ]
        );
    }

    #[test]
    fn dynamic_abi_registration_preserves_external_connection_owner() {
        struct OwnedCallback;
        impl MessageHandler for OwnedCallback {
            fn on_message(&mut self, _: &AppMessage, _: &mut MessageBus) {}
            fn needs_poll(&self) -> bool {
                true
            }
            fn poll(&mut self, ctx: &mut PollContext<'_>) {
                ctx.register_owned_dynamic_command(
                    "external".into(),
                    String::new(),
                    String::new(),
                    String::new(),
                    42,
                );
            }
        }
        unsafe extern "C" fn register(
            handle: HostRegistrarHandle,
            callbacks: *const HostCallbacks,
        ) -> AbiStatus {
            // SAFETY: The fixture is called by the host with its live registrar.
            let mut registrar = unsafe { PluginRegistrar::from_abi(handle, callbacks) }.unwrap();
            registrar.set_metadata(PluginMetadata::new("owned", "1.0", "Owned callback"));
            registrar.set_message_handler(OwnedCallback);
            registrar.finish()
        }
        let mut host = PluginHost::new();
        let mut executor = CommandExecutor::new();
        let mut declaration = test_declaration(Some(register));
        declaration.capabilities |= CAPABILITY_MESSAGE_RUNTIME;
        load_declaration_for_test(&mut host, &mut executor, declaration).unwrap();
        host.poll_all(&SharedFixture::new().shared(), &mut MessageBus::new());
        assert_eq!(host.pending_registrations.len(), 1);
        assert_eq!(host.pending_registrations[0].executor, "owned");
        assert_eq!(host.pending_registrations[0].owner_tag, Some(42));
    }

    #[test]
    fn pending_wait_detects_a_later_serial_executor_dependency() {
        use patinae_cmd::tasks::TaskSpec;
        let mut host = host(true);
        let mut kernel = patinae_framework::kernel::AppKernel::new();
        let waiter = kernel
            .tasks
            .admit(TaskSpec::new("python", "first"))
            .unwrap();
        kernel.tasks.started(waiter, "first").unwrap();
        let target = kernel.tasks.admit(TaskSpec::new("pml", "native")).unwrap();
        let fixture = SharedFixture::new();
        request(
            &mut host,
            &mut kernel.bus,
            "first",
            vec![WireHostQuery::WaitTask {
                id: 71,
                task_id: target,
                waiter: Some(waiter),
                timeout_ms: None,
            }],
        );
        host.poll_all(&fixture.shared(), &mut kernel.bus);
        host.apply_task_controls(&mut kernel, None, (1, 1));
        host.poll_all(&fixture.shared(), &mut kernel.bus);
        assert!(replies(&mut kernel.bus).is_empty());
        let mut child = TaskSpec::new("python", "first");
        child.parent_id = Some(target);
        kernel.tasks.admit(child).unwrap();
        host.apply_task_controls(&mut kernel, None, (1, 1));
        host.poll_all(&fixture.shared(), &mut kernel.bus);
        let responses = replies(&mut kernel.bus);
        assert_eq!(responses.len(), 1);
        assert!(matches!(&responses[0].1.result,
            Ok(WireHostQueryValue::TaskWait(Err(error))) if error.code == "would_deadlock"));
    }

    #[test]
    fn extra_deliveries_do_not_age_idle_atom_streams() {
        use patinae_framework::atom_stream::{
            AtomStreamMode, AtomStreamPlan, AtomStreamRequest, AtomStreamScope,
        };
        let mut host = host(true);
        let fixture = SharedFixture::new();
        let shared = fixture.shared();
        let request = AtomStreamRequest {
            scope: AtomStreamScope::All,
            mode: AtomStreamMode::Read,
            columns: Vec::new(),
            chunk_size: 1,
        };
        host.plugins[0].atom_streams.streams.insert(
            1,
            crate::plugin::AtomStreamState {
                plan: AtomStreamPlan::open(&shared, &request).unwrap(),
                position: 0,
                chunk_size: 1,
                idle_polls: 0,
            },
        );
        let mut bus = MessageBus::new();
        host.poll_all(&shared, &mut bus);
        for _ in 0..patinae_cmd::tasks::ASYNC_TASK_BATCH_SIZE {
            host.poll_task_deliveries(&shared, &mut bus);
        }
        assert_eq!(host.plugins[0].atom_streams.streams[&1].idle_polls, 1);
        host.poll_all(&shared, &mut bus);
        assert_eq!(host.plugins[0].atom_streams.streams[&1].idle_polls, 2);
    }

    #[test]
    fn only_ready_task_deliveries_request_another_poll() {
        use patinae_plugin::wire::WireHostQueryValue;
        let mut host = host(true);
        assert!(!host.has_ready_task_delivery());
        for (value, expected) in [
            (
                WireHostQueryValue::TaskSnapshot(Err(TaskLookupError::InvalidId)),
                false,
            ),
            (
                WireHostQueryValue::TaskList(Err(TaskLookupError::InvalidId)),
                false,
            ),
            (WireHostQueryValue::TaskAcknowledged(Ok(true)), true),
            (
                WireHostQueryValue::TaskWait(Err(patinae_cmd::tasks::TaskError::new(
                    "timeout", "timeout",
                ))),
                true,
            ),
            (
                WireHostQueryValue::TaskCommand(Ok(patinae_cmd::CommandReply {
                    result: Ok(()),
                    messages: Vec::new(),
                    task_ids: Vec::new(),
                })),
                true,
            ),
        ] {
            host.host_query_results = vec![vec![WireHostQueryResult {
                id: 1,
                result: Ok(value),
            }]];
            assert_eq!(host.has_ready_task_delivery(), expected);
        }
        host.host_query_results.clear();
        assert!(!host.has_ready_task_delivery());
    }

    #[test]
    fn admitted_invocation_reaches_executor_on_first_poll_exactly_once() {
        let mut host = host(true);
        let mut kernel = patinae_framework::kernel::AppKernel::new();
        kernel.executor.register_task_executor("first");
        let mut request = patinae_cmd::PluginTaskRequest::new("script", serde_json::json!({}));
        request.executor = "first".into();
        let task_id = kernel.start_plugin_task(request, None).unwrap();
        let fixture = SharedFixture::new();
        for expected in [1, 0] {
            host.prepare_task_dispatch(&mut kernel);
            assert_eq!(host.has_ready_task_delivery(), expected != 0);
            let mut shared = fixture.shared();
            shared.tasks = Some(&kernel.tasks);
            host.poll_all(&shared, &mut kernel.bus);
            let invocations: Vec<_> = kernel
                .bus
                .drain_outbox()
                .into_iter()
                .filter_map(|message| {
                    if let AppMessage::Custom { topic, payload } = message {
                        if topic.ends_with(":invocation") {
                            return Some((topic, wire::decode::<TaskId>(&payload).unwrap()));
                        }
                    }
                    None
                })
                .collect();
            assert_eq!(invocations.len(), expected);
            if expected == 1 {
                assert_eq!(invocations[0], ("first:invocation".into(), task_id));
            }
            host.apply_task_controls(&mut kernel, None, (1, 1));
        }
    }

    #[test]
    fn dynamic_producer_admission_events_and_child_commands_are_owner_checked() {
        use patinae_cmd::tasks::{TaskEffects, TaskEvent, TaskOutcome, TaskState};
        let mut host = host(true);
        let mut kernel = patinae_framework::kernel::AppKernel::new();
        kernel.executor.register_task_executor("first");
        kernel.executor.register_task_executor("second");
        let fixture = SharedFixture::new();
        let mut description =
            patinae_cmd::PluginTaskRequest::new("script", serde_json::json!({"code":"pass"}));
        description.executor = "second".into();
        request(
            &mut host,
            &mut kernel.bus,
            "first",
            vec![WireHostQuery::StartTask {
                id: 71,
                request: description,
                parent_id: None,
            }],
        );
        let mut shared = fixture.shared();
        shared.tasks = Some(&kernel.tasks);
        host.poll_all(&shared, &mut kernel.bus);
        host.apply_task_controls(&mut kernel, None, (1, 1));
        let task_id = match &host.host_query_results[0][0].result {
            Ok(WireHostQueryValue::TaskStarted(Ok(id))) => *id,
            _ => panic!("expected admission receipt"),
        };
        assert!(kernel.tasks.is_owned_by(task_id, "first"));
        assert_eq!(kernel.tasks.get(task_id).unwrap().state, TaskState::Queued);
        host.pending_task_controls.push((
            1,
            WireHostQuery::TaskEvent {
                id: 72,
                task_id,
                event: TaskEvent::Finished(TaskOutcome::success(None, TaskEffects::None)),
            },
        ));
        host.pending_task_controls.push((
            0,
            WireHostQuery::TaskCommand {
                id: 73,
                task_id,
                command: "group acknowledged".into(),
                silent: true,
            },
        ));
        host.apply_task_controls(&mut kernel, None, (1, 1));
        assert!(
            matches!(&host.host_query_results[1][0].result, Ok(WireHostQueryValue::TaskAcknowledged(Err(error))) if error.code == "wrong_executor")
        );
        assert!(kernel.session.registry.contains("acknowledged"));
        assert!(!kernel.tasks.get(task_id).unwrap().state.is_terminal());
        host.pending_task_controls.push((
            0,
            WireHostQuery::TaskEvent {
                id: 74,
                task_id,
                event: TaskEvent::Finished(TaskOutcome::success(None, TaskEffects::None)),
            },
        ));
        host.apply_task_controls(&mut kernel, None, (1, 1));
        assert_eq!(
            kernel.tasks.get(task_id).unwrap().state,
            TaskState::Succeeded
        );
        // Group commands still use unrestricted registry access. Completion
        // is acknowledged without claiming exact mutation evidence.
        assert_eq!(
            kernel.tasks.get(task_id).unwrap().effects,
            TaskEffects::Unknown
        );
    }

    #[test]
    fn connection_loss_only_finishes_that_executors_tasks() {
        let mut host = host(false);
        let mut kernel = patinae_framework::kernel::AppKernel::new();
        let executor = host.plugins[0].metadata.name.clone();
        kernel.executor.register_task_executor(&executor);
        let mut request = patinae_cmd::PluginTaskRequest::new("callback", serde_json::json!({}));
        request.executor = executor;
        request.owner_tag = Some(1);
        let old = kernel.start_plugin_task(request.clone(), None).unwrap();
        request.owner_tag = Some(2);
        let current = kernel.start_plugin_task(request, None).unwrap();
        host.pending_task_controls.push((
            0,
            WireHostQuery::FailTaskOwner {
                id: 1,
                owner_tag: 1,
            },
        ));
        host.apply_task_controls(&mut kernel, None, (1, 1));
        assert_eq!(
            kernel.tasks.get(old).unwrap().state,
            patinae_cmd::tasks::TaskState::Failed
        );
        assert_eq!(
            kernel.tasks.get(current).unwrap().state,
            patinae_cmd::tasks::TaskState::Queued
        );
    }

    #[test]
    fn static_task_controls_are_deferred_and_owned() {
        check_owner_routing(false);
    }

    #[test]
    fn dynamic_task_controls_are_deferred_and_owned() {
        check_owner_routing(true);
    }

    #[test]
    fn dynamic_get_and_list_keep_structured_lookup_results() {
        let mut host = host(true);
        let tasks = patinae_framework::tasks::native_task_runner();
        let task_id = tasks
            .admit(patinae_cmd::tasks::TaskSpec::new("test", "test"))
            .unwrap();
        let missing_id = TaskId::new(task_id.instance(), task_id.sequence() + 1);
        let fixture = SharedFixture::new();
        let mut shared = fixture.shared();
        shared.tasks = Some(&tasks);
        let mut bus = MessageBus::new();
        request(
            &mut host,
            &mut bus,
            "first",
            vec![
                WireHostQuery::GetTask { id: 1, task_id },
                WireHostQuery::GetTask {
                    id: 2,
                    task_id: missing_id,
                },
                WireHostQuery::ListTasks {
                    id: 3,
                    request: TaskListRequest::default(),
                },
            ],
        );
        host.poll_all(&shared, &mut bus);
        host.poll_all(&shared, &mut bus);
        let replies = replies(&mut bus);
        assert_eq!(replies.len(), 3);
        assert!(
            matches!(&replies[0].1.result, Ok(WireHostQueryValue::TaskSnapshot(Ok(snapshot))) if snapshot.id == task_id && snapshot.outcome.is_none())
        );
        assert!(matches!(
            &replies[1].1.result,
            Ok(WireHostQueryValue::TaskSnapshot(Err(
                TaskLookupError::NotFound
            )))
        ));
        assert!(
            matches!(&replies[2].1.result, Ok(WireHostQueryValue::TaskList(Ok(page))) if page.tasks.len() == 1 && page.tasks[0].id == task_id)
        );
    }
}
