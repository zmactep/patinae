use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use patinae_cmd::{
    CommandExecutor, DynamicCommand, DynamicCommandInvocation, DynamicSettingRegistry,
};
use patinae_framework::atom_stream::{
    AtomChunk, AtomStreamMode, AtomStreamPlan, AtomStreamRequest, AtomStreamScope,
    ATOM_STREAM_CHUNK_TARGET_BYTES, DEFAULT_ATOM_STREAM_CHUNK_ROWS, MAX_ATOM_STREAM_CHUNK_ROWS,
};
use patinae_framework::component::SharedContext;
use patinae_framework::message::{AppMessage, MessageBus};
use patinae_mol::{Atom, AtomIndex, Element, SecondaryStructure};
use patinae_plugin::registrar::{CommandExecRequest, PluginKeyAction, PollContext, ViewerMutation};
use patinae_plugin::wire::{
    self, WireAtomStreamOpened, WireHostQuery, WireHostQueryResult, WireHostQueryValue,
    WirePollSharedInput, WireViewerAction, WireViewportImageSummary, RUNTIME_WIRE_VERSION,
};
use patinae_scene::{label_object_view, parse_key_string, KeyBinding, ViewerLike, ViewportImage};

use crate::host::{PluginHost, TriggeredHotkey};
use crate::panic::panic_payload_to_string;
use crate::plugin::{AtomStreamState, AtomStreams, LoadedPlugin};
use crate::CommandResult;

const ATOM_STREAM_IDLE_POLL_LIMIT: u32 = 600;

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

    pub fn poll_all(&mut self, shared: &SharedContext<'_>, bus: &mut MessageBus) {
        self.process_panel_events(shared, bus);

        let invocations: Vec<DynamicCommandInvocation> = self
            .dynamic_invocations
            .lock()
            .map(|mut v| std::mem::take(&mut *v))
            .unwrap_or_default();
        let results = std::mem::take(&mut self.command_results);
        let previous_query_results = std::mem::take(&mut self.host_query_results);
        let triggered = std::mem::take(&mut self.triggered_hotkeys);
        let poll_shared = poll_shared_input_from_context(shared);
        let triggered_by_plugin = triggered_hotkeys_by_plugin(triggered, self.plugins.len());

        let mut exec_queue = Vec::new();
        let mut reg_queue = Vec::new();
        let mut unreg_queue = Vec::new();
        let mut notification_queue = Vec::new();
        let mut hotkey_reg_queue = Vec::new();
        let mut hotkey_unreg_queue = Vec::new();
        let mut mutation_queue = Vec::new();
        let mut viewer_action_queue = Vec::new();
        let mut panel_update_requested = false;
        let mut next_query_results = Vec::with_capacity(self.plugins.len());

        for (plugin_index, plugin) in self.plugins.iter_mut().enumerate() {
            expire_idle_atom_streams(&mut plugin.atom_streams);
            let query_results = previous_query_results
                .get(plugin_index)
                .map_or(&[][..], Vec::as_slice);
            let triggered = triggered_by_plugin
                .get(plugin_index)
                .map_or(&[][..], Vec::as_slice);
            let mut host_query_queue = Vec::new();
            let mut ctx = PollContext::new(
                shared,
                &poll_shared,
                bus,
                &results,
                query_results,
                &invocations,
                triggered,
                &self.plugin_dirs,
                &mut exec_queue,
                &mut reg_queue,
                &mut unreg_queue,
                &mut notification_queue,
                &mut hotkey_reg_queue,
                &mut hotkey_unreg_queue,
                &mut mutation_queue,
                &mut host_query_queue,
                &mut viewer_action_queue,
                &mut panel_update_requested,
            );

            run_triggered_hotkeys(plugin, triggered, &mut ctx);
            poll_handler(plugin, &mut ctx);
            next_query_results.push(resolve_host_queries(
                &host_query_queue,
                shared,
                &mut plugin.atom_streams,
            ));
        }

        for action in viewer_action_queue {
            apply_local_viewer_action(action, &mut mutation_queue, &mut panel_update_requested);
        }

        self.pending_executions = exec_queue;
        self.pending_registrations = reg_queue;
        self.pending_unregistrations = unreg_queue;
        self.notification_messages = notification_queue;
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
                        if let Ok(mut list) = self.dynamic_invocations.lock() {
                            list.push(DynamicCommandInvocation {
                                name: name.clone(),
                                args: args.clone(),
                            });
                        }
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

        let invocations = self.invocations_handle();
        for reg in std::mem::take(&mut self.pending_registrations) {
            let command = DynamicCommand::new(
                reg.name,
                reg.description,
                reg.usage,
                reg.arguments,
                invocations.clone(),
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

    pub fn take_pending_executions(&mut self) -> Vec<CommandExecRequest> {
        std::mem::take(&mut self.pending_executions)
    }

    pub fn store_command_results(&mut self, results: Vec<CommandResult>) {
        self.command_results = results;
    }

    pub fn take_pending_mutations(&mut self) -> Vec<ViewerMutation> {
        std::mem::take(&mut self.pending_mutations)
    }

    pub fn notification_messages(&self) -> &[String] {
        &self.notification_messages
    }
}

fn poll_shared_input_from_context(ctx: &SharedContext<'_>) -> WirePollSharedInput {
    WirePollSharedInput {
        wire_version: RUNTIME_WIRE_VERSION,
        scene_generation: ctx.scene_generation,
        object_names: ctx.registry.names().map(ToOwned::to_owned).collect(),
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

fn apply_local_viewer_action(
    action: WireViewerAction,
    mutation_queue: &mut Vec<ViewerMutation>,
    panel_update_requested: &mut bool,
) {
    match action {
        WireViewerAction::SetViewportImage(image) => {
            mutation_queue.push(Box::new(move |viewer| {
                viewer.set_viewport_image(Some(image));
            }));
        }
        WireViewerAction::ClearViewportImage => {
            mutation_queue.push(Box::new(|viewer| {
                viewer.set_viewport_image(None);
            }));
        }
        WireViewerAction::RequestRedraw => {
            mutation_queue.push(Box::new(|viewer| viewer.request_redraw()));
        }
        WireViewerAction::RequestPanelUpdate => {
            *panel_update_requested = true;
        }
        WireViewerAction::ApplyAtomPropertyChanges(changes) => {
            mutation_queue.push(Box::new(move |viewer| {
                apply_atom_property_change_batch(viewer, &changes);
            }));
        }
    }
}

pub(crate) fn apply_atom_property_change_batch<V: ViewerLike + ?Sized>(
    viewer: &mut V,
    changes: &[wire::WireAtomPropertyChange],
) -> bool {
    let mut applied = false;
    let mut changed = false;
    let mut identity_changed = false;
    for change in changes {
        let Some(molecule) = viewer.objects_mut().get_molecule_mut(&change.object) else {
            continue;
        };
        let Some(atom) = molecule
            .molecule_mut()
            .get_atom_mut(AtomIndex(change.atom_index))
        else {
            continue;
        };
        let outcome = apply_atom_property_changes(atom, &change.changes);
        applied |= outcome.applied;
        changed |= outcome.changed;
        identity_changed |= outcome.identity_changed;
    }
    if identity_changed {
        viewer.reconcile_recent_atoms();
    }
    if changed {
        viewer.request_redraw();
    }
    applied
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AtomPropertyChangeOutcome {
    applied: bool,
    changed: bool,
    identity_changed: bool,
}

fn apply_atom_property_changes(
    atom: &mut Atom,
    changes: &[(String, wire::WireAtomPropertyValue)],
) -> AtomPropertyChangeOutcome {
    let mut outcome = AtomPropertyChangeOutcome::default();
    for (key, value) in changes {
        let change = apply_atom_property_change(atom, key, value);
        outcome.applied |= change.applied;
        outcome.changed |= change.changed;
        outcome.identity_changed |= change.identity_changed;
    }
    outcome
}

fn apply_atom_property_change(
    atom: &mut Atom,
    key: &str,
    value: &wire::WireAtomPropertyValue,
) -> AtomPropertyChangeOutcome {
    let (changed, identity_changed) = match (key, value) {
        ("name", wire::WireAtomPropertyValue::Str(value)) => {
            let changed = atom.name.as_ref() != value.as_str();
            if changed {
                atom.name = Arc::from(value.as_str());
            }
            (changed, changed)
        }
        ("b", wire::WireAtomPropertyValue::F32(value)) => {
            let changed = atom.b_factor.to_bits() != value.to_bits();
            if changed {
                atom.b_factor = *value;
            }
            (changed, false)
        }
        ("q", wire::WireAtomPropertyValue::F32(value)) => {
            let changed = atom.occupancy.to_bits() != value.to_bits();
            if changed {
                atom.occupancy = *value;
            }
            (changed, false)
        }
        ("vdw", wire::WireAtomPropertyValue::F32(value)) => {
            let changed = atom.vdw.to_bits() != value.to_bits();
            if changed {
                atom.vdw = *value;
            }
            (changed, false)
        }
        ("partial_charge", wire::WireAtomPropertyValue::F32(value)) => {
            let changed = atom.partial_charge.to_bits() != value.to_bits();
            if changed {
                atom.partial_charge = *value;
            }
            (changed, false)
        }
        ("formal_charge", wire::WireAtomPropertyValue::I8(value)) => {
            let changed = atom.formal_charge != *value;
            if changed {
                atom.formal_charge = *value;
            }
            (changed, false)
        }
        ("color", wire::WireAtomPropertyValue::I32(value)) => {
            let changed = atom.repr.colors.base != *value;
            if changed {
                atom.repr.colors.base = *value;
            }
            (changed, false)
        }
        ("elem", wire::WireAtomPropertyValue::Str(value)) => {
            let Some(element) = Element::from_symbol(value) else {
                return AtomPropertyChangeOutcome::default();
            };
            let changed = atom.element != element;
            if changed {
                atom.element = element;
            }
            (changed, false)
        }
        ("ss", wire::WireAtomPropertyValue::Str(value)) => {
            let ss_type = match value.as_str() {
                "H" => SecondaryStructure::Helix,
                "S" => SecondaryStructure::Sheet,
                _ => SecondaryStructure::Loop,
            };
            let changed = atom.ss_type != ss_type;
            if changed {
                atom.ss_type = ss_type;
            }
            (changed, false)
        }
        ("type", wire::WireAtomPropertyValue::Str(value)) => {
            let hetatm = value == "HETATM";
            let changed = atom.state.hetatm != hetatm;
            if changed {
                atom.state.hetatm = hetatm;
            }
            (changed, false)
        }
        ("alt", wire::WireAtomPropertyValue::Str(value)) => {
            let alt = value.chars().next().unwrap_or(' ');
            let changed = atom.alt != alt;
            if changed {
                atom.alt = alt;
            }
            (changed, changed)
        }
        ("chain", wire::WireAtomPropertyValue::Str(value)) => {
            let changed = atom.residue.key.chain.as_str() != value.as_str();
            if changed {
                let mut residue = (*atom.residue).clone();
                residue.key.chain = value.clone();
                atom.residue = Arc::new(residue);
            }
            (changed, changed)
        }
        ("resn", wire::WireAtomPropertyValue::Str(value)) => {
            let changed = atom.residue.key.resn.as_str() != value.as_str();
            if changed {
                let mut residue = (*atom.residue).clone();
                residue.key.resn = value.clone();
                atom.residue = Arc::new(residue);
            }
            (changed, changed)
        }
        ("resv", wire::WireAtomPropertyValue::I32(value)) => {
            let changed = atom.residue.key.resv != *value;
            if changed {
                let mut residue = (*atom.residue).clone();
                residue.key.resv = *value;
                atom.residue = Arc::new(residue);
            }
            (changed, changed)
        }
        ("segi", wire::WireAtomPropertyValue::Str(value)) => {
            let changed = atom.residue.segi.as_str() != value.as_str();
            if changed {
                let mut residue = (*atom.residue).clone();
                residue.segi = value.clone();
                atom.residue = Arc::new(residue);
            }
            (changed, changed)
        }
        _ => return AtomPropertyChangeOutcome::default(),
    };
    AtomPropertyChangeOutcome {
        applied: true,
        changed,
        identity_changed,
    }
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
mod recent_atom_tests {
    use super::*;
    use patinae_mol::{AtomBuilder, ObjectMolecule};
    use patinae_scene::{MoleculeObject, PickHit, Session, SessionAdapter};
    use patinae_settings::groups::RecentPickLimit;

    #[test]
    fn atom_identity_outcome_ignores_no_op_assignments() {
        let mut atom = AtomBuilder::new()
            .name("CA")
            .element_symbol("C")
            .resn("GLY")
            .resv(1)
            .chain("A")
            .build();

        let no_op = apply_atom_property_changes(
            &mut atom,
            &[(
                "name".to_string(),
                wire::WireAtomPropertyValue::Str("CA".to_string()),
            )],
        );
        assert_eq!(
            no_op,
            AtomPropertyChangeOutcome {
                applied: true,
                changed: false,
                identity_changed: false,
            }
        );

        let changed = apply_atom_property_changes(
            &mut atom,
            &[(
                "chain".to_string(),
                wire::WireAtomPropertyValue::Str("B".to_string()),
            )],
        );
        assert_eq!(
            changed,
            AtomPropertyChangeOutcome {
                applied: true,
                changed: true,
                identity_changed: true,
            }
        );
    }

    #[test]
    fn plugin_identity_changes_reconcile_recent_atoms_but_visual_changes_do_not() {
        let mut molecule = ObjectMolecule::new("obj");
        molecule.add_atom(
            AtomBuilder::new()
                .name("CA")
                .element_symbol("C")
                .resn("GLY")
                .resv(1)
                .chain("A")
                .build(),
        );
        let mut session = Session::new();
        session
            .registry
            .add(MoleculeObject::with_name(molecule, "obj"));
        let path = patinae_scene::canonical_atom_path_for_hit(
            &PickHit {
                object_name: "obj".to_string(),
                object_type: patinae_scene::ObjectType::Molecule,
                atom_index: Some(AtomIndex(0)),
                position: Default::default(),
                distance: 0.0,
            },
            session.registry.get_molecule("obj").unwrap().molecule(),
        )
        .unwrap();
        session
            .recent_atoms
            .insert(path, RecentPickLimit::Unlimited);
        let mut needs_redraw = false;
        let mut viewer = SessionAdapter {
            session: &mut session,
            render_context: None,
            default_size: (64, 64),
            needs_redraw: &mut needs_redraw,
            async_fetch_fn: None,
        };

        assert!(apply_atom_property_change_batch(
            &mut viewer,
            &[wire::WireAtomPropertyChange {
                object: "obj".to_string(),
                atom_index: 0,
                changes: vec![(
                    "name".to_string(),
                    wire::WireAtomPropertyValue::Str("CA".to_string()),
                )],
            }],
        ));
        assert!(!*viewer.needs_redraw);
        assert_eq!(viewer.session().recent_atoms.len(), 1);

        apply_atom_property_change_batch(
            &mut viewer,
            &[wire::WireAtomPropertyChange {
                object: "obj".to_string(),
                atom_index: 0,
                changes: vec![("b".to_string(), wire::WireAtomPropertyValue::F32(5.0))],
            }],
        );
        assert!(*viewer.needs_redraw);
        assert_eq!(viewer.session().recent_atoms.len(), 1);

        apply_atom_property_change_batch(
            &mut viewer,
            &[wire::WireAtomPropertyChange {
                object: "obj".to_string(),
                atom_index: 0,
                changes: vec![(
                    "name".to_string(),
                    wire::WireAtomPropertyValue::Str("CB".to_string()),
                )],
            }],
        );
        assert!(viewer.session().recent_atoms.is_empty());
    }
}
