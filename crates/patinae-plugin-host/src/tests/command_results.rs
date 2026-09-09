//! Command round trips through the production host and SDK ABI adapters.

use super::*;
use crate::CommandResult;
use patinae_cmd::{CmdError, CmdResult, Command, MessageKind, ParsedCommand, ViewerLike};
use patinae_framework::kernel::AppKernel;
use patinae_framework::model::output::OutputKind;
use patinae_plugin::ffi::CAPABILITY_MESSAGE_RUNTIME;
use patinae_plugin::wire::{WireCommandExecRequest, WireCommandResult};

struct ReportingCommand;

impl Command for ReportingCommand {
    fn name(&self) -> &str {
        "third_party_report"
    }
    fn description(&self) -> &str {
        "Third-party report fixture"
    }
    fn help(&self) -> &str {
        "Third-party report fixture\nUsage: third_party_report [fail|defer]"
    }
    fn execute<'v, 'r>(
        &self,
        ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
        args: &ParsedCommand,
    ) -> CmdResult {
        ctx.print("first\nsecond line");
        ctx.print_warning("warning");
        ctx.print_error("diagnostic");
        ctx.print("last");
        ctx.show_panel("fixture-panel");
        match args
            .args
            .first()
            .map(|(_, value)| value.to_string())
            .as_deref()
        {
            Some("fail") => Err(CmdError::execution("fixture failure")),
            Some("defer") => {
                ctx.mark_deferred();
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

struct Requester {
    name: &'static str,
    pending: Vec<WireCommandExecRequest>,
}

impl MessageHandler for Requester {
    fn on_message(&mut self, message: &AppMessage, _bus: &mut MessageBus) {
        if let AppMessage::Custom { topic, payload } = message {
            if topic == self.name {
                self.pending
                    .extend(wire::decode::<Vec<WireCommandExecRequest>>(payload).unwrap());
            }
        }
    }
    fn needs_poll(&self) -> bool {
        true
    }
    fn poll(&mut self, ctx: &mut PollContext<'_>) {
        for request in self.pending.drain(..) {
            ctx.execute_command(request.id, &request.command, request.silent);
        }
        for result in ctx.command_results {
            ctx.bus.send(AppMessage::Custom {
                topic: format!("{}:response", self.name),
                payload: wire::encode(&WireCommandResult {
                    id: result.id,
                    result: result.result.clone(),
                    messages: result.messages.clone(),
                    deferred: result.deferred,
                })
                .unwrap(),
            });
        }
    }
}

unsafe fn register_requester(
    handle: HostRegistrarHandle,
    callbacks: *const HostCallbacks,
    name: &'static str,
) -> AbiStatus {
    // SAFETY: Forwarded unchanged from the host registration callback.
    let Ok(mut registrar) = (unsafe { PluginRegistrar::from_abi(handle, callbacks) }) else {
        return AbiStatus::INVALID;
    };
    registrar.set_metadata(PluginMetadata::new(name, "1.0", "Round-trip fixture"));
    registrar.register_command(ReportingCommand);
    registrar.set_message_handler(Requester {
        name,
        pending: Vec::new(),
    });
    registrar.finish()
}

unsafe extern "C" fn register_first(
    handle: HostRegistrarHandle,
    callbacks: *const HostCallbacks,
) -> AbiStatus {
    // SAFETY: Inputs are provided by PluginHost for this call.
    unsafe { register_requester(handle, callbacks, "first") }
}

unsafe extern "C" fn register_second(
    handle: HostRegistrarHandle,
    callbacks: *const HostCallbacks,
) -> AbiStatus {
    // SAFETY: Inputs are provided by PluginHost for this call.
    unsafe { register_requester(handle, callbacks, "second") }
}

struct Harness {
    host: PluginHost,
    kernel: AppKernel,
    shared: SharedFixture,
    panel_actions: usize,
}

impl Harness {
    fn new(two_plugins: bool) -> Self {
        let mut kernel = AppKernel::new();
        let mut host = PluginHost::new();
        let callbacks: &[PluginRegisterFn] = if two_plugins {
            &[register_first, register_second]
        } else {
            &[register_first]
        };
        for &register in callbacks {
            let mut declaration = test_declaration(Some(register));
            declaration.capabilities |= CAPABILITY_MESSAGE_RUNTIME;
            load_declaration_for_test(&mut host, &mut kernel.executor, declaration).unwrap();
        }
        Self {
            host,
            kernel,
            shared: SharedFixture::new(),
            panel_actions: 0,
        }
    }

    fn request(&mut self, plugin: &str, requests: &[(u64, &str, bool)]) {
        let requests: Vec<_> = requests
            .iter()
            .map(|&(id, command, silent)| WireCommandExecRequest {
                id,
                command: command.into(),
                silent,
            })
            .collect();
        self.host.broadcast(
            &AppMessage::Custom {
                topic: plugin.into(),
                payload: wire::encode(&requests).unwrap(),
            },
            &mut self.kernel.bus,
        );
    }

    fn poll(&mut self) {
        self.host
            .poll_all(&self.shared.shared(), &mut self.kernel.bus);
    }

    fn execute(&mut self) -> Vec<CommandResult> {
        self.host
            .take_pending_executions()
            .into_iter()
            .map(|request| {
                let execution = self.kernel.execute_command_captured(
                    &request.command,
                    request.silent,
                    None,
                    (64, 64),
                );
                CommandResult::from_execution(request.id, execution)
            })
            .collect()
    }

    fn responses(&mut self) -> Vec<(String, WireCommandResult)> {
        self.kernel
            .bus
            .drain_outbox()
            .into_iter()
            .filter_map(|message| {
                if matches!(&message, AppMessage::ShowPanel(_)) {
                    self.panel_actions += 1;
                }
                if let AppMessage::Custom { topic, payload } = message {
                    if topic.ends_with(":response") {
                        return Some((topic, wire::decode(&payload).unwrap()));
                    }
                }
                None
            })
            .collect()
    }

    fn round_trip(&mut self, command: &str, silent: bool) -> WireCommandResult {
        self.request("first", &[(42, command, silent)]);
        self.poll();
        let results = self.execute();
        self.host.store_command_results(results);
        self.poll();
        let mut responses = self.responses();
        assert_eq!(responses.len(), 1);
        let (topic, response) = responses.remove(0);
        assert_eq!(topic, "first:response");
        assert_eq!(response.id, 42);
        response
    }
}

fn message_pairs(result: &WireCommandResult) -> Vec<(MessageKind, &str)> {
    result
        .messages
        .iter()
        .map(|m| (m.kind, m.text.as_str()))
        .collect()
}

#[test]
fn typed_messages_survive_abi_round_trip_and_actions_run_once() {
    let mut h = Harness::new(false);
    h.request("first", &[(7, "third_party_report", false)]);
    h.poll();
    let results = h.execute();
    assert_eq!(
        h.kernel
            .bus
            .drain_outbox()
            .iter()
            .filter(|msg| matches!(msg, AppMessage::ShowPanel(id) if id == "fixture-panel"))
            .count(),
        1
    );
    h.host.store_command_results(results);
    h.poll();
    let responses = h.responses();
    assert_eq!(
        h.panel_actions, 0,
        "result delivery must not replay actions"
    );
    assert_eq!(responses.len(), 1);
    let response = &responses[0].1;
    assert!(response.result.is_ok());
    assert!(!response.deferred);
    assert_eq!(
        message_pairs(response),
        vec![
            (MessageKind::Info, "first\nsecond line"),
            (MessageKind::Warning, "warning"),
            (MessageKind::Error, "diagnostic"),
            (MessageKind::Info, "last"),
        ]
    );
    for message in &response.messages {
        assert!(h
            .kernel
            .output
            .buffer
            .iter()
            .any(|line| line.text == message.text));
    }
    h.poll();
    assert!(h.kernel.bus.drain_outbox().is_empty());
}

#[test]
fn execution_error_retains_partial_messages_and_does_not_apply_actions() {
    let mut h = Harness::new(false);
    let response = h.round_trip("third_party_report fail", false);
    assert_eq!(h.panel_actions, 0, "failed command actions must not run");
    assert!(response
        .result
        .as_ref()
        .unwrap_err()
        .contains("fixture failure"));
    assert_eq!(response.messages.len(), 5);
    assert_eq!(response.messages[0].text, "first\nsecond line");
    let last = response.messages.last().unwrap();
    assert_eq!(last.kind, MessageKind::Error);
    assert_eq!(&last.text, response.result.as_ref().unwrap_err());
    assert!(!response.deferred);
    assert_eq!(
        h.kernel
            .output
            .buffer
            .iter()
            .filter(|line| line.text.contains("fixture failure"))
            .count(),
        1
    );
}

#[test]
fn help_uses_the_registered_third_party_command() {
    let mut h = Harness::new(false);
    let expected = h
        .kernel
        .execute_command("help third_party_report", false, None, (64, 64))
        .unwrap();
    let response = h.round_trip("help third_party_report", true);
    assert!(response.result.is_ok());
    assert_eq!(response.messages.len(), expected.messages.len());
    for (actual, expected) in response.messages.iter().zip(expected.messages) {
        assert_eq!(actual.text, expected.text);
        assert_eq!(actual.kind, expected.kind);
    }
    assert!(response
        .messages
        .iter()
        .any(|msg| msg.text.contains("Third-party report fixture")));
}

#[test]
fn capabilities_uses_the_live_executor() {
    let mut h = Harness::new(false);
    for command in ["capabilities", "capabilities plugins"] {
        let expected = h
            .kernel
            .execute_command(command, false, None, (64, 64))
            .unwrap();
        let response = h.round_trip(command, true);
        assert!(response.result.is_ok());
        assert_eq!(response.messages[0].text, expected.messages[0].text);
    }
    let response = h.round_trip("capabilities plugins", true);
    assert!(response.messages[0].text.contains("first"));
}

#[test]
fn request_ids_are_local_to_each_plugin_and_results_can_arrive_in_batches() {
    let mut h = Harness::new(true);
    h.request(
        "first",
        &[
            (7, "help third_party_report", true),
            (8, "capabilities", true),
        ],
    );
    h.request("second", &[(7, "third_party_report fail", true)]);
    h.poll();
    let mut results = h.execute();
    assert_eq!(results.len(), 3);
    assert_ne!(results[0].id, results[2].id);
    let second = results.pop().unwrap();
    h.host.store_command_results(vec![second]);
    // A later poll must not lose outstanding requests or broadcast the response.
    h.poll();
    let responses = h.responses();
    assert_eq!(responses.len(), 1);
    assert_eq!(
        (&*responses[0].0, responses[0].1.id),
        ("second:response", 7)
    );
    assert!(responses[0].1.result.is_err());
    results.reverse();
    for result in results {
        h.host.store_command_results(vec![result]);
    }
    h.poll();
    let responses = h.responses();
    assert_eq!(
        responses
            .iter()
            .map(|(name, r)| (name.as_str(), r.id))
            .collect::<Vec<_>>(),
        vec![("first:response", 8), ("first:response", 7)]
    );
    assert!(responses[0].1.messages[0].text.contains("Capabilities:"));
    h.poll();
    assert!(h.responses().is_empty());
}

#[test]
fn silence_only_changes_ui_output() {
    let mut h = Harness::new(false);
    let response = h.round_trip("third_party_report", true);
    assert_eq!(response.messages.len(), 4);
    assert_eq!(h.kernel.output.buffer.len(), 1);
    assert_eq!(h.kernel.output.buffer[0].kind, OutputKind::Error);
    assert_eq!(h.kernel.output.buffer[0].text, "diagnostic");
}

#[test]
fn deferred_work_is_not_reported_as_completed() {
    let mut h = Harness::new(false);
    // Explicit deferral crosses the command ABI as well as the poll ABI.
    let response = h.round_trip("third_party_report defer", true);
    assert!(response.result.is_ok());
    assert!(response.deferred);
    h.kernel
        .executor
        .registry_mut()
        .register(patinae_cmd::DynamicCommand::new(
            "queued_fixture".into(),
            "queued".into(),
            "queued_fixture".into(),
            String::new(),
            h.host.invocations_handle(),
        ));
    let response = h.round_trip("queued_fixture", true);
    assert!(response.result.is_ok());
    assert!(response.deferred);
}

#[test]
fn accepted_native_async_task_is_deferred() {
    use patinae_framework::tasks::{AsyncTask, TaskResult};
    use std::future::Future;
    use std::pin::Pin;
    struct AcceptedTask;
    impl TaskResult for AcceptedTask {
        fn apply(self: Box<Self>, _kernel: &mut AppKernel) {}
    }
    impl AsyncTask for AcceptedTask {
        fn notification_message(&self) -> String {
            "fixture task".into()
        }
        fn execute(self: Box<Self>) -> Pin<Box<dyn Future<Output = Box<dyn TaskResult>> + Send>> {
            Box::pin(async { Box::new(AcceptedTask) as Box<dyn TaskResult> })
        }
    }
    let mut h = Harness::new(false);
    h.kernel
        .set_async_command_handler(Some(Box::new(|_| Some(Box::new(AcceptedTask)))));
    struct AsyncRequestCommand;
    impl Command for AsyncRequestCommand {
        fn name(&self) -> &str {
            "async_fixture"
        }
        fn execute<'v, 'r>(
            &self,
            ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
            _args: &ParsedCommand,
        ) -> CmdResult {
            let request = patinae_cmd::AsyncCommandRequest::Fetch(patinae_cmd::FetchRequest {
                code: "1abc".into(),
                name: "fixture".into(),
                format: patinae_cmd::FetchFormatCode::Cif,
                bond_tolerance: 0.45,
                auto_dss: false,
                dss_algorithm: Default::default(),
            });
            assert!(ctx.submit_async_request(request));
            ctx.print("queued");
            Ok(())
        }
    }
    h.kernel
        .executor
        .registry_mut()
        .register(AsyncRequestCommand);
    let response = h.round_trip("async_fixture", true);

    assert!(response.result.is_ok(), "{:?}", response.result);
    assert!(response.deferred);
    assert!(!response.messages.is_empty());
    h.poll();
    assert!(h.responses().is_empty());
}

#[test]
fn queued_requests_survive_an_extra_poll_and_duplicate_results_are_ignored() {
    let mut h = Harness::new(false);
    h.request("first", &[(0, "capabilities", true)]);
    h.poll();
    h.poll();
    let results = h.execute();
    assert_eq!(results.len(), 1);
    h.host.store_command_results(results.clone());
    h.host.store_command_results(results);
    h.poll();
    let responses = h.responses();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].1.id, 0);
}

#[test]
fn old_wire_output_is_rejected_before_applying_messages_or_actions() {
    let mut session = Session::new();
    let mut needs_redraw = false;
    let mut viewer = SessionAdapter {
        session: &mut session,
        render_context: None,
        default_size: (64, 64),
        needs_redraw: &mut needs_redraw,
        async_fetch_fn: None,
    };
    let mut ctx = CommandContext::new(&mut viewer);
    let output = WireCommandOutput {
        wire_version: RUNTIME_WIRE_VERSION - 1,
        deferred: true,
        result: Ok(()),
        output: vec![OutputMessage::info("must not apply")],
        actions: vec![patinae_cmd::CommandAction::Quit],
        session: Vec::new(),
        viewport_image: None,
        viewport_image_changed: false,
    };
    let error =
        apply_command_output(&mut ctx, output, CommandRuntimeRequirements::NONE).unwrap_err();
    assert!(error.to_string().contains("wire version mismatch"));
    assert!(ctx.take_output().is_empty());
    assert!(ctx.take_actions().is_empty());
    assert!(!ctx.is_deferred());
}
