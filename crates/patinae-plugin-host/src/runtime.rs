use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::actions::apply_local_viewer_action;
use patinae_cmd::{CommandExecutor, DynamicCommand, DynamicSettingRegistry};
use patinae_framework::atom_stream::{
    AtomChunk, AtomStreamMode, AtomStreamPlan, AtomStreamRequest, AtomStreamScope,
    ATOM_STREAM_CHUNK_TARGET_BYTES, DEFAULT_ATOM_STREAM_CHUNK_ROWS, MAX_ATOM_STREAM_CHUNK_ROWS,
};
use patinae_framework::component::SharedContext;
use patinae_framework::message::{AppMessage, MessageBus};
use patinae_plugin::registrar::{CommandExecRequest, PluginKeyAction, PollContext, ViewerMutation};
use patinae_plugin::wire::{
    self, WireAtomStreamOpened, WireHostQuery, WireHostQueryResult, WireHostQueryValue,
    WirePollSharedInput, WireViewportImageSummary, RUNTIME_WIRE_VERSION,
};
use patinae_scene::{label_object_view, parse_key_string, KeyBinding, ViewportImage};

use crate::host::{PluginHost, TriggeredHotkey};
use crate::panic::panic_payload_to_string;
use crate::plugin::{AtomStreamState, AtomStreams, LoadedPlugin};
use crate::CommandResult;

// Expire streams after idle host ticks, independently of task delivery passes.
const ATOM_STREAM_IDLE_POLL_LIMIT: u32 = 600;

mod task_controls;
pub(crate) use task_controls::PendingTaskWait;

impl PluginHost {
    pub fn broadcast(&mut self, msg: &AppMessage, bus: &mut MessageBus) {
        for plugin in &mut self.plugins {
            if plugin.faulted {
                continue;
            }
            if let Some(handler) = &mut plugin.message_handler {
                let result = catch_unwind(AssertUnwindSafe(|| handler.on_message(msg, bus)));
                if let Err(panic_info) = result {
                    log::error!(
                        "Plugin '{}' panicked during on_message: {}. Plugin disabled.",
                        plugin.metadata.name,
                        panic_payload_to_string(&panic_info),
                    );
                    plugin.faulted = true;
                }
            }
        }
    }

    /// Queue admitted invocations before polling their executors.
    ///
    /// This transfers transport messages only; task state stays in the kernel.
    pub fn prepare_task_dispatch(&mut self, kernel: &mut patinae_framework::kernel::AppKernel) {
        self.task_invocations.extend(kernel.take_task_invocations());
    }

    /// Whether ready task deliveries can unblock an executor or waiting client.
    ///
    /// Snapshot and list replies deliberately do not request another pass:
    /// observers may ask for them on every poll, including while idle.
    pub fn has_ready_task_delivery(&self) -> bool {
        !self.task_invocations.is_empty()
            || self.host_query_results.iter().flatten().any(|reply| {
                matches!(
                    &reply.result,
                    Ok(WireHostQueryValue::TaskStarted(_)
                        | WireHostQueryValue::TaskAcknowledged(_)
                        | WireHostQueryValue::TaskCommand(_)
                        | WireHostQueryValue::TaskWait(_)
                        | WireHostQueryValue::TaskCancel(_))
                )
            })
    }

    pub fn poll_all(&mut self, shared: &SharedContext<'_>, bus: &mut MessageBus) {
        self.poll_plugins(shared, bus, true);
    }

    /// Deliver queued task replies within the current host tick.
    ///
    /// Extra delivery passes must not advance idle stream expiry counters.
    pub fn poll_task_deliveries(&mut self, shared: &SharedContext<'_>, bus: &mut MessageBus) {
        self.poll_plugins(shared, bus, false);
    }

    fn poll_plugins(
        &mut self,
        shared: &SharedContext<'_>,
        bus: &mut MessageBus,
        advance_idle: bool,
    ) {
        self.process_panel_events(shared, bus);

        let invocations = std::mem::take(&mut self.task_invocations);
        let results = std::mem::take(&mut self.command_results);
        let previous_query_results = std::mem::take(&mut self.host_query_results);
        let triggered = std::mem::take(&mut self.triggered_hotkeys);
        let poll_shared = poll_shared_input_from_context(shared);
        let triggered_by_plugin = triggered_hotkeys_by_plugin(triggered, self.plugins.len());

        let mut exec_queue = Vec::new();
        let mut reg_queue = Vec::new();
        let mut unreg_queue = Vec::new();
        let mut hotkey_reg_queue = Vec::new();
        let mut hotkey_unreg_queue = Vec::new();
        let mut mutation_queue = Vec::new();
        let mut viewer_action_queue = Vec::new();
        let mut panel_update_requested = false;
        let mut next_query_results = Vec::with_capacity(self.plugins.len());

        for (plugin_index, plugin) in self.plugins.iter_mut().enumerate() {
            let mut plugin_exec_queue = Vec::new();
            let mut plugin_reg_queue = Vec::new();
            if advance_idle {
                expire_idle_atom_streams(&mut plugin.atom_streams);
            }
            let query_results = previous_query_results
                .get(plugin_index)
                .map_or(&[][..], Vec::as_slice);
            let triggered = triggered_by_plugin
                .get(plugin_index)
                .map_or(&[][..], Vec::as_slice);
            let mut host_query_queue = Vec::new();
            let plugin_invocations: Vec<_> = invocations
                .iter()
                .filter(|invocation| invocation.request.executor == plugin.metadata.name)
                .cloned()
                .collect();
            let mut ctx = PollContext::new(
                shared,
                &poll_shared,
                bus,
                results.get(plugin_index).map_or(&[][..], Vec::as_slice),
                query_results,
                &plugin_invocations,
                triggered,
                &self.plugin_dirs,
                &mut plugin_exec_queue,
                &mut plugin_reg_queue,
                &mut unreg_queue,
                &mut hotkey_reg_queue,
                &mut hotkey_unreg_queue,
                &mut mutation_queue,
                &mut host_query_queue,
                &mut viewer_action_queue,
                &mut panel_update_requested,
            );

            ctx.task_cancellations = shared
                .tasks
                .map(|tasks| {
                    tasks
                        .cancellation_targets()
                        .into_iter()
                        .filter(|(_, owner)| {
                            owner == &plugin.metadata.name
                                || owner.starts_with(&format!("{}/", plugin.metadata.name))
                        })
                        .map(|(id, _)| id)
                        .collect()
                })
                .unwrap_or_default();
            run_triggered_hotkeys(plugin, triggered, &mut ctx);

            poll_handler(plugin, &mut ctx);
            if ctx.executor_failure.is_some() {
                plugin.faulted = true;
            }
            for mut registration in plugin_reg_queue {
                registration.executor = plugin.metadata.name.clone();
                reg_queue.push(registration);
            }
            host_query_queue.retain(|query| {
                if matches!(
                    query,
                    WireHostQuery::ForgetTaskWait { .. }
                        | WireHostQuery::CancelTask { .. }
                        | WireHostQuery::FailTaskOwner { .. }
                        | WireHostQuery::StartTask { .. }
                        | WireHostQuery::TaskEvent { .. }
                        | WireHostQuery::TaskAction { .. }
                        | WireHostQuery::TaskCommand { .. }
                        | WireHostQuery::WaitTask { .. }
                ) {
                    self.pending_task_controls
                        .push((plugin_index, query.clone()));
                    false
                } else {
                    true
                }
            });
            next_query_results.push(resolve_host_queries(
                &host_query_queue,
                shared,
                &mut plugin.atom_streams,
            ));
            for mut request in plugin_exec_queue {
                // Frontends see a host token; plugins retain their own ID namespace.
                while self.command_owners.contains_key(&self.next_command_id) {
                    self.next_command_id = self.next_command_id.wrapping_add(1);
                }
                let token = self.next_command_id;
                self.next_command_id = self.next_command_id.wrapping_add(1);
                self.command_owners
                    .insert(token, (plugin_index, request.id));
                request.id = token;
                exec_queue.push(request);
            }
        }

        for action in viewer_action_queue {
            apply_local_viewer_action(action, &mut mutation_queue, &mut panel_update_requested);
        }

        self.pending_executions.extend(exec_queue);
        self.pending_registrations = reg_queue;
        self.pending_unregistrations = unreg_queue;
        self.pending_hotkey_registrations = hotkey_reg_queue;
        self.pending_hotkey_unregistrations = hotkey_unreg_queue;
        self.pending_mutations = mutation_queue;
        self.host_query_results = next_query_results;
        if panel_update_requested {
            self.bump_panel_ui_generation();
        }
    }

    pub fn handle_hotkey(&mut self, binding: KeyBinding, bus: &mut MessageBus) -> bool {
        for (plugin_index, plugin) in self.plugins.iter_mut().enumerate() {
            if plugin.faulted {
                continue;
            }
            if let Some(action) = plugin.hotkeys.get(&binding) {
                match action {
                    PluginKeyAction::Command(command) => {
                        bus.execute_command(command.clone());
                    }
                    PluginKeyAction::DynamicCommand { name, args } => {
                        let arguments = args
                            .iter()
                            .map(|arg| format!("\"{}\"", arg.replace('"', "\\\"")))
                            .collect::<Vec<_>>()
                            .join(", ");
                        bus.execute_command(format!("{name} {arguments}"));
                    }
                    PluginKeyAction::Custom { topic, payload } => {
                        bus.send(AppMessage::Custom {
                            topic: topic.clone(),
                            payload: payload.clone(),
                        });
                    }
                    PluginKeyAction::Callback(_) => {
                        self.triggered_hotkeys.push(TriggeredHotkey {
                            plugin_index,
                            binding,
                        });
                    }
                }
                return true;
            }
        }
        false
    }

    pub fn apply_dynamic_command_changes(&mut self, executor: &mut CommandExecutor) -> bool {
        let mut changed = false;
        for name in std::mem::take(&mut self.pending_unregistrations) {
            changed |= executor.registry_mut().unregister(&name);
        }

        for reg in std::mem::take(&mut self.pending_registrations) {
            let command = DynamicCommand::new(
                reg.name,
                reg.description,
                reg.usage,
                reg.arguments,
                reg.executor,
                reg.owner_tag,
            );
            executor.registry_mut().register_boxed(Box::new(command));
            changed = true;
        }
        changed
    }

    pub fn apply_hotkey_changes(&mut self) {
        let registrations = std::mem::take(&mut self.pending_hotkey_registrations);
        let unregistrations = std::mem::take(&mut self.pending_hotkey_unregistrations);

        for key_str in &unregistrations {
            if let Ok(key) = parse_key_string(key_str) {
                for plugin in &mut self.plugins {
                    plugin.hotkeys.unbind(key);
                }
            }
        }

        if let Some(plugin) = self.plugins.last_mut() {
            for (key_str, action) in registrations {
                match parse_key_string(&key_str) {
                    Ok(key) => plugin.hotkeys.bind(key, action),
                    Err(e) => log::warn!("Invalid hotkey string '{}': {}", key_str, e),
                }
            }
        }
    }

    /// Drain requests whose IDs are opaque host routing tokens.
    ///
    /// Return each result with the unchanged token to [`Self::store_command_results`].
    pub fn take_pending_executions(&mut self) -> Vec<CommandExecRequest> {
        std::mem::take(&mut self.pending_executions)
    }

    pub fn store_command_results(&mut self, results: Vec<CommandResult>) {
        self.command_results
            .resize_with(self.plugins.len(), Vec::new);
        for mut result in results {
            if let Some((plugin_index, original_id)) = self.command_owners.remove(&result.id) {
                result.id = original_id;
                self.command_results[plugin_index].push(result);
            } else {
                log::warn!(
                    "Ignoring command result with unknown host token {}",
                    result.id
                );
            }
        }
    }

    pub fn take_pending_mutations(&mut self) -> Vec<ViewerMutation> {
        std::mem::take(&mut self.pending_mutations)
    }
}

fn poll_shared_input_from_context(ctx: &SharedContext<'_>) -> WirePollSharedInput {
    let mut selection_names = ctx.selections.names();
    selection_names.sort_unstable();
    WirePollSharedInput {
        wire_version: RUNTIME_WIRE_VERSION,
        scene_generation: ctx.scene_generation,
        object_names: ctx.registry.names().map(ToOwned::to_owned).collect(),
        selection_names,
        pick_paths: ctx.recent_atoms.paths().map(ToOwned::to_owned).collect(),
        camera: ctx.camera.clone(),
        movie: patinae_scene::MovieStateSnapshot {
            frame_count: ctx.movie.effective_frame_count(),
            current_frame: ctx.movie.current_frame(),
            is_playing: ctx.movie.is_playing(),
            rock_enabled: ctx.movie.is_rock_enabled(),
        },
        settings: ctx.settings.clone(),
        clear_color: ctx.clear_color,
        viewport_image: ctx.viewport_image.map(viewport_image_summary),
        command_names: ctx.command_names.to_vec(),
        setting_names: ctx
            .setting_names
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
        dynamic_settings: dynamic_settings_to_wire(ctx.dynamic_settings),
    }
}

fn dynamic_settings_to_wire(
    dynamic_settings: Option<&DynamicSettingRegistry>,
) -> Vec<wire::WireDynamicSetting> {
    let Some(dynamic_settings) = dynamic_settings else {
        return Vec::new();
    };
    dynamic_settings
        .names()
        .iter()
        .filter_map(|name| {
            let entry = dynamic_settings.lookup(name)?;
            let value = entry
                .store
                .read()
                .ok()
                .and_then(|store| store.get(name).cloned());
            Some(wire::WireDynamicSetting {
                descriptor: (&entry.descriptor).into(),
                value,
            })
        })
        .collect()
}

fn viewport_image_summary(image: &ViewportImage) -> WireViewportImageSummary {
    WireViewportImageSummary {
        width: image.width,
        height: image.height,
        len: image.data.len(),
        signature: viewport_image_signature(image),
    }
}

fn viewport_image_signature(image: &ViewportImage) -> u64 {
    let mut signature = image.width as u64;
    signature = mix_signature(signature, image.height as u64);
    signature = mix_signature(signature, image.data.len() as u64);
    signature = mix_signature(signature, image.data.as_ptr() as usize as u64);
    if let Some(first) = image.data.first() {
        signature = mix_signature(signature, *first as u64);
    }
    if let Some(last) = image.data.last() {
        signature = mix_signature(signature, *last as u64);
    }
    signature
}

fn mix_signature(acc: u64, value: u64) -> u64 {
    acc.rotate_left(13)
        .wrapping_mul(0x9E37_79B1_85EB_CA87)
        .wrapping_add(value)
}

fn resolve_host_queries(
    queries: &[WireHostQuery],
    shared: &SharedContext<'_>,
    atom_streams: &mut AtomStreams,
) -> Vec<WireHostQueryResult> {
    queries
        .iter()
        .map(|query| resolve_host_query(query, shared, atom_streams))
        .collect()
}

fn resolve_host_query(
    query: &WireHostQuery,
    shared: &SharedContext<'_>,
    atom_streams: &mut AtomStreams,
) -> WireHostQueryResult {
    match query {
        WireHostQuery::TaskConfig { id } => WireHostQueryResult {
            id: *id,
            result: shared
                .tasks
                .map(|tasks| WireHostQueryValue::TaskConfig(tasks.config().clone()))
                .ok_or_else(|| "task runtime unavailable".into()),
        },
        WireHostQuery::GetTask { id, task_id } => WireHostQueryResult {
            id: *id,
            result: shared
                .tasks
                .map(|tasks| WireHostQueryValue::TaskSnapshot(tasks.get(*task_id)))
                .ok_or_else(|| "host task queries are unavailable".to_owned()),
        },
        WireHostQuery::ListTasks { id, request } => WireHostQueryResult {
            id: *id,
            result: shared
                .tasks
                .map(|tasks| WireHostQueryValue::TaskList(tasks.list(request)))
                .ok_or_else(|| "host task queries are unavailable".to_owned()),
        },
        WireHostQuery::ForgetTaskWait { id }
        | WireHostQuery::FailTaskOwner { id, .. }
        | WireHostQuery::StartTask { id, .. }
        | WireHostQuery::TaskEvent { id, .. }
        | WireHostQuery::TaskAction { id, .. }
        | WireHostQuery::TaskCommand { id, .. }
        | WireHostQuery::WaitTask { id, .. } => WireHostQueryResult {
            id: *id,
            result: Err("task control requires host mutation phase".into()),
        },
        WireHostQuery::CancelTask { id, .. } => WireHostQueryResult {
            id: *id,
            result: Err("task controls must run after the host read phase".to_owned()),
        },
        WireHostQuery::ObjectNames { id } => WireHostQueryResult {
            id: *id,
            result: Ok(WireHostQueryValue::ObjectNames(
                shared.registry.names().map(ToOwned::to_owned).collect(),
            )),
        },
        WireHostQuery::View { id } => WireHostQueryResult {
            id: *id,
            result: Ok(WireHostQueryValue::View(shared.camera.current_view())),
        },
        WireHostQuery::CountAtoms { id, selection } => WireHostQueryResult {
            id: *id,
            result: count_atoms(shared, selection).map(WireHostQueryValue::CountAtoms),
        },
        WireHostQuery::ViewportImage { id } => WireHostQueryResult {
            id: *id,
            result: Ok(WireHostQueryValue::ViewportImage(
                shared.viewport_image.cloned(),
            )),
        },
        WireHostQuery::LabelObject { id, name } => WireHostQueryResult {
            id: *id,
            result: Ok(WireHostQueryValue::LabelObject(label_object_view(
                shared.registry,
                shared.settings,
                shared.named_palette,
                name,
            ))),
        },
        WireHostQuery::OpenAtomStream { id, request } => WireHostQueryResult {
            id: *id,
            result: open_atom_stream(atom_streams, shared, request)
                .map(WireHostQueryValue::AtomStreamOpened),
        },
        WireHostQuery::ReadAtomStream {
            id,
            stream_id,
            max_rows,
        } => WireHostQueryResult {
            id: *id,
            result: read_atom_stream(atom_streams, shared, *stream_id, *max_rows)
                .map(WireHostQueryValue::AtomStreamChunk),
        },
        WireHostQuery::CloseAtomStream { id, stream_id } => WireHostQueryResult {
            id: *id,
            result: close_atom_stream(atom_streams, *stream_id)
                .map(|()| WireHostQueryValue::AtomStreamClosed),
        },
    }
}

fn count_atoms(shared: &SharedContext<'_>, selection: &str) -> Result<usize, String> {
    let request = AtomStreamRequest {
        scope: AtomStreamScope::Selection(selection.to_string()),
        mode: AtomStreamMode::Read,
        columns: Vec::new(),
        chunk_size: DEFAULT_ATOM_STREAM_CHUNK_ROWS,
    };
    AtomStreamPlan::open(shared, &request).map(|plan| plan.total_count)
}

fn expire_idle_atom_streams(atom_streams: &mut AtomStreams) {
    for stream in atom_streams.streams.values_mut() {
        stream.idle_polls = stream.idle_polls.saturating_add(1);
    }
    atom_streams
        .streams
        .retain(|_, stream| stream.idle_polls <= ATOM_STREAM_IDLE_POLL_LIMIT);
}

fn open_atom_stream(
    atom_streams: &mut AtomStreams,
    shared: &SharedContext<'_>,
    request: &patinae_framework::atom_stream::AtomStreamRequest,
) -> Result<WireAtomStreamOpened, String> {
    let plan = AtomStreamPlan::open(shared, request)?;
    let total_count = plan.total_count;
    let stream_id = next_atom_stream_id(atom_streams);
    atom_streams.streams.insert(
        stream_id,
        AtomStreamState {
            plan,
            position: 0,
            chunk_size: request.bounded_chunk_size(),
            idle_polls: 0,
        },
    );
    Ok(WireAtomStreamOpened {
        stream_id,
        total_count,
    })
}

fn next_atom_stream_id(atom_streams: &mut AtomStreams) -> u64 {
    atom_streams.next_id = atom_streams.next_id.wrapping_add(1).max(1);
    while atom_streams.streams.contains_key(&atom_streams.next_id) {
        atom_streams.next_id = atom_streams.next_id.wrapping_add(1).max(1);
    }
    atom_streams.next_id
}

fn read_atom_stream(
    atom_streams: &mut AtomStreams,
    shared: &SharedContext<'_>,
    stream_id: u64,
    max_rows: usize,
) -> Result<AtomChunk, String> {
    let (chunk, remove_stream) = {
        let stream = atom_streams
            .streams
            .get_mut(&stream_id)
            .ok_or_else(|| format!("atom stream {stream_id} is not open"))?;
        stream.idle_polls = 0;
        let requested_rows = if max_rows == 0 {
            stream.chunk_size
        } else {
            max_rows
        };
        let row_limit = requested_rows.clamp(1, MAX_ATOM_STREAM_CHUNK_ROWS);
        let chunk = limited_atom_chunk(&stream.plan, shared, stream.position, row_limit)?;
        stream.position += chunk.rows.len();
        let done = chunk.done;
        (chunk, done)
    };
    if remove_stream {
        atom_streams.streams.remove(&stream_id);
    }
    Ok(chunk)
}

fn limited_atom_chunk(
    plan: &AtomStreamPlan,
    shared: &SharedContext<'_>,
    start: usize,
    max_rows: usize,
) -> Result<AtomChunk, String> {
    let mut rows = max_rows.clamp(1, MAX_ATOM_STREAM_CHUNK_ROWS);
    loop {
        let chunk = plan.chunk(shared, start, rows)?;
        let encoded_len = wire::encode(&chunk)?.len();
        if encoded_len <= ATOM_STREAM_CHUNK_TARGET_BYTES || rows <= 1 {
            return Ok(chunk);
        }
        rows = (rows / 2).max(1);
    }
}

fn close_atom_stream(atom_streams: &mut AtomStreams, stream_id: u64) -> Result<(), String> {
    atom_streams
        .streams
        .remove(&stream_id)
        .map(|_| ())
        .ok_or_else(|| format!("atom stream {stream_id} is not open"))
}

fn triggered_hotkeys_by_plugin(
    triggered: Vec<TriggeredHotkey>,
    plugin_count: usize,
) -> Vec<Vec<KeyBinding>> {
    let mut by_plugin = vec![Vec::new(); plugin_count];
    for hotkey in triggered {
        if let Some(bindings) = by_plugin.get_mut(hotkey.plugin_index) {
            bindings.push(hotkey.binding);
        }
    }
    by_plugin
}

fn run_triggered_hotkeys(
    plugin: &mut LoadedPlugin,
    triggered: &[KeyBinding],
    ctx: &mut PollContext<'_>,
) {
    for binding in triggered {
        if plugin.faulted {
            continue;
        }
        if let Some(PluginKeyAction::Callback(cb)) = plugin.hotkeys.get_mut(binding) {
            let result = catch_unwind(AssertUnwindSafe(|| cb(ctx)));
            if let Err(panic_info) = result {
                log::error!(
                    "Plugin '{}' panicked in hotkey callback: {}. Plugin disabled.",
                    plugin.metadata.name,
                    panic_payload_to_string(&panic_info),
                );
                plugin.faulted = true;
            }
        }
    }
}

fn poll_handler(plugin: &mut LoadedPlugin, ctx: &mut PollContext<'_>) {
    if plugin.faulted {
        return;
    }
    if let Some(handler) = &mut plugin.message_handler {
        if handler.needs_poll() {
            let result = catch_unwind(AssertUnwindSafe(|| handler.poll(ctx)));
            if let Err(panic_info) = result {
                log::error!(
                    "Plugin '{}' panicked during poll: {}. Plugin disabled.",
                    plugin.metadata.name,
                    panic_payload_to_string(&panic_info),
                );
                plugin.faulted = true;
            }
        }
    }
}

#[cfg(test)]
mod command_results {
    //! Command round trips through the production host and SDK ABI adapters.

    use crate::host::tests::{test_declaration, SharedFixture};
    use crate::loader::apply_command_output;
    use crate::loader::load_declaration_for_test;
    use crate::CommandResult;
    use crate::PluginHost;
    use patinae_cmd::CommandContext;
    use patinae_cmd::CommandRuntimeRequirements;
    use patinae_cmd::OutputMessage;
    use patinae_cmd::{CmdError, CmdResult, Command, MessageKind, ParsedCommand, ViewerLike};
    use patinae_framework::kernel::AppKernel;
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
    use patinae_plugin::wire;
    use patinae_plugin::wire::WireCommandOutput;
    use patinae_plugin::wire::RUNTIME_WIRE_VERSION;
    use patinae_plugin::wire::{WireCommandExecRequest, WireCommandResult};
    use patinae_scene::Session;
    use patinae_scene::SessionAdapter;

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
            if args
                .args
                .first()
                .is_some_and(|(_, value)| value.to_string() == "markdown")
            {
                ctx.print_markdown("# Result\n\n**formatted**");
                ctx.print("**literal**");
                return Ok(());
            }
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
                Some("background") => ctx.request_task(patinae_cmd::AsyncCommandRequest::Plugin(
                    patinae_cmd::PluginTaskRequest::new("fixture", serde_json::json!({})),
                )),
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
                    payload: wire::encode(result).unwrap(),
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
    fn markdown_survives_command_and_requester_abi_round_trips() {
        for silent in [false, true] {
            let mut h = Harness::new(false);
            let response = h.round_trip("third_party_report markdown", silent);
            assert!(response.result.is_ok());
            assert_eq!(response.messages.len(), 2);
            assert_eq!(
                response.messages[0].format,
                patinae_cmd::OutputFormat::Markdown
            );
            assert_eq!(response.messages[0].text, "# Result\n\n**formatted**");
            assert_eq!(response.messages[1].format, patinae_cmd::OutputFormat::Text);
            if silent {
                assert!(h.kernel.output.buffer.is_empty());
            } else {
                assert!(h.kernel.output.buffer.iter().any(|message| {
                    message.format == patinae_cmd::OutputFormat::Markdown
                        && message.text == response.messages[0].text
                }));
            }
        }
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
        assert!(h.kernel.output.buffer.is_empty());
        assert!(response
            .messages
            .iter()
            .any(|message| message.text == "diagnostic"));
    }

    #[test]
    fn background_work_returns_host_issued_id() {
        let mut h = Harness::new(false);
        // A description crosses the command ABI and receives its host identity.
        let response = h.round_trip("third_party_report background", true);
        assert_eq!(response.task_ids.len(), 1);
        assert!(response.result.is_ok());
        h.kernel
            .executor
            .registry_mut()
            .register(patinae_cmd::DynamicCommand::new(
                "queued_fixture".into(),
                "queued".into(),
                "queued_fixture".into(),
                String::new(),
                "first".into(),
                None,
            ));
        let response = h.round_trip("queued_fixture", true);
        assert!(response.result.is_ok());
    }

    #[test]
    fn accepted_native_async_task_returns_receipt() {
        use patinae_framework::tasks::{AsyncTask, TaskResult};

        use std::future::Future;
        use std::pin::Pin;
        struct AcceptedTask;
        impl TaskResult for AcceptedTask {
            fn apply(
                self: Box<Self>,
                _kernel: &mut AppKernel,
                _id: patinae_cmd::tasks::TaskId,
            ) -> patinae_cmd::tasks::TaskOutcome {
                patinae_cmd::tasks::TaskOutcome::success(
                    None,
                    patinae_cmd::tasks::TaskEffects::None,
                )
            }
        }
        impl AsyncTask for AcceptedTask {
            fn notification_message(&self) -> String {
                "fixture task".into()
            }
            fn execute(
                self: Box<Self>,
            ) -> Pin<Box<dyn Future<Output = Box<dyn TaskResult>> + Send>> {
                Box::pin(async { Box::new(AcceptedTask) as Box<dyn TaskResult> })
            }
        }
        let mut h = Harness::new(false);
        h.kernel
            .set_async_command_handler(Some(Box::new(|_, _| Some(Box::new(AcceptedTask)))));
        struct AsyncRequestCommand;
        impl Command for AsyncRequestCommand {
            fn name(&self) -> &str {
                "async_fixture"
            }
            fn execute<'v, 'r>(
                &self,
                ctx: &mut CommandContext<'v, 'r, dyn ViewerLike + 'v>,
                args: &ParsedCommand,
            ) -> CmdResult {
                let request = patinae_cmd::AsyncCommandRequest::Fetch(patinae_cmd::FetchRequest {
                    code: "1abc".into(),
                    name: "fixture".into(),
                    format: patinae_cmd::FetchFormatCode::Cif,
                    bond_tolerance: 0.45,
                    auto_dss: false,
                    dss_algorithm: Default::default(),
                });
                assert!(matches!(
                    ctx.submit_async_request(request),
                    patinae_cmd::AsyncCommandAcceptance::Accepted(_)
                ));
                ctx.print("queued");
                if args.args.is_empty() {
                    Ok(())
                } else {
                    Err(CmdError::execution("failed after accepting task"))
                }
            }
        }
        h.kernel
            .executor
            .registry_mut()
            .register(AsyncRequestCommand);
        let response = h.round_trip("async_fixture", true);

        assert!(response.result.is_ok(), "{:?}", response.result);
        assert!(!response.messages.is_empty());
        assert_eq!(response.task_ids.len(), 1);
        assert!(h.kernel.tasks.get(response.task_ids[0]).is_ok());
        let failed = h.round_trip("async_fixture fail", true);
        assert!(failed.result.is_err());
        assert_eq!(failed.task_ids.len(), 1);
        assert_ne!(failed.task_ids, response.task_ids);
        assert!(h.kernel.tasks.get(failed.task_ids[0]).is_ok());
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
        };
        let mut ctx = CommandContext::new(&mut viewer);
        let output = WireCommandOutput {
            task_requests: Vec::new(),
            wire_version: RUNTIME_WIRE_VERSION - 1,
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
    }
}
