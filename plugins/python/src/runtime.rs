use std::sync::{atomic::AtomicBool, Arc, Mutex};

use patinae_plugin::prelude::PluginRegistrar;

use crate::commands::{AlterCommand, IterateCommand, PythonCommand};
use crate::handler::PythonHandler;
use crate::panel::PythonScriptPanel;
use crate::shared::SharedState;
use crate::worker;
use patinae_plugin::prelude::AsyncCommandRequest;

pub(crate) fn register(reg: &mut PluginRegistrar) {
    let interrupt_requested = Arc::new(AtomicBool::new(false));

    let shared_state = Arc::new(Mutex::new(SharedState::new(interrupt_requested.clone())));
    let panel_state = crate::panel::shared_panel_state();

    let (worker_handle, result_rx) = worker::spawn_worker();

    reg.register_command(PythonCommand);
    reg.register_command(IterateCommand);
    reg.register_command(AlterCommand);
    reg.register_panel(PythonScriptPanel::new(panel_state.clone()));
    reg.register_script_handler("py", move |path: &str| {
        Ok(AsyncCommandRequest::Plugin(crate::commands::python_task(
            serde_json::json!({"path": path, "origin": "script"}),
        )))
    });

    reg.set_message_handler(PythonHandler::new(
        worker_handle,
        shared_state,
        result_rx,
        panel_state,
    ));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use patinae_plugin::ffi::*;
    use patinae_plugin::prelude::{MessageHandler, ParsedCommand};
    use patinae_plugin::wire::{self, *};
    use patinae_scene::Session;
    use std::ffi::c_void;

    #[derive(Default)]
    pub(crate) struct AbiHarness {
        commands: Vec<(String, PluginCommandHandle, AbiCommandVTable)>,
        scripts: Vec<(PluginScriptHandlerHandle, AbiScriptHandlerVTable)>,
        panels: Vec<(PluginPanelHandle, AbiPanelVTable)>,
        handler: Option<AbiMessageHandlerDescriptor>,
    }

    // SAFETY: All callbacks receive a pointer to the live harness passed by
    // `register`; descriptor pointers are borrowed only during that call.
    unsafe extern "C" fn command(
        handle: HostRegistrarHandle,
        descriptor: *const AbiCommandDescriptor,
    ) -> AbiStatus {
        // SAFETY: The registrar lends the live harness for this callback.
        let harness = unsafe { &mut *handle.0.cast::<AbiHarness>() };
        // SAFETY: The SDK lends a valid descriptor for this callback.
        let descriptor = unsafe { &*descriptor };
        // SAFETY: Descriptor strings remain valid until this callback returns.
        let bytes = unsafe { descriptor.name.as_bytes_checked(MAX_ABI_STRING_LEN) }.unwrap();
        harness.commands.push((
            String::from_utf8(bytes.to_vec()).unwrap(),
            descriptor.handle,
            descriptor.vtable,
        ));
        AbiStatus::OK
    }

    unsafe extern "C" fn script(
        handle: HostRegistrarHandle,
        descriptor: *const AbiScriptHandlerDescriptor,
    ) -> AbiStatus {
        // SAFETY: See the callback lifetime contract above.
        let harness = unsafe { &mut *handle.0.cast::<AbiHarness>() };
        // SAFETY: The SDK lends a valid descriptor for this callback.
        let descriptor = unsafe { &*descriptor };
        harness.scripts.push((descriptor.handle, descriptor.vtable));
        AbiStatus::OK
    }

    unsafe extern "C" fn panel(
        handle: HostRegistrarHandle,
        descriptor: *const AbiPanelDescriptor,
    ) -> AbiStatus {
        // SAFETY: See the callback lifetime contract above.
        let harness = unsafe { &mut *handle.0.cast::<AbiHarness>() };
        // SAFETY: The SDK lends a valid descriptor for this callback.
        let descriptor = unsafe { &*descriptor };
        harness.panels.push((descriptor.handle, descriptor.vtable));
        AbiStatus::OK
    }

    unsafe extern "C" fn handler(
        handle: HostRegistrarHandle,
        descriptor: *const AbiMessageHandlerDescriptor,
    ) -> AbiStatus {
        // SAFETY: The registrar and descriptor remain live during this callback.
        unsafe {
            (*handle.0.cast::<AbiHarness>()).handler = Some(*descriptor);
        }
        AbiStatus::OK
    }

    unsafe extern "C" fn sink(handle: *mut c_void, bytes: AbiU8Slice) -> AbiStatus {
        // SAFETY: Callers provide a live Vec and the SDK lends this byte slice
        // until the synchronous sink invocation returns.
        let output = unsafe { &mut *handle.cast::<Vec<u8>>() };
        // SAFETY: The SDK lends this initialized byte slice until the sink returns.
        let bytes = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) };
        output.extend_from_slice(bytes);
        AbiStatus::OK
    }

    impl AbiHarness {
        fn register(&mut self, register: impl FnOnce(&mut PluginRegistrar<'_>)) {
            let callbacks = HostCallbacks {
                table_version: HOST_CALLBACKS_VERSION,
                register_metadata: None,
                register_command: Some(command),
                register_panel: Some(panel),
                register_setting: None,
                register_message_handler: Some(handler),
                register_script_handler: Some(script),
                register_format_handler: None,
                register_hotkey: None,
                report_unsupported: None,
            };
            // SAFETY: Both pointers remain live until registration completes.
            let mut registrar = unsafe {
                PluginRegistrar::from_abi(
                    HostRegistrarHandle((self as *mut Self).cast()),
                    &callbacks,
                )
            }
            .unwrap();
            register(&mut registrar);
            assert!(registrar.finish().is_ok());
        }

        pub(crate) fn with_handler(handler: impl MessageHandler + 'static) -> Self {
            let mut harness = Self::default();
            harness.register(|registrar| registrar.set_message_handler(handler));
            harness
        }

        pub(crate) fn poll(&mut self, input: &WirePollInput) -> WirePollOutput {
            let descriptor = self.handler.unwrap();
            let bytes = wire::encode(input).unwrap();
            let mut output = Vec::<u8>::new();
            // SAFETY: The harness owns this runtime handle; buffers and output
            // are live throughout the synchronous ABI call.
            let status = unsafe {
                (descriptor.vtable.poll.unwrap())(
                    descriptor.handle,
                    AbiU8Slice {
                        ptr: bytes.as_ptr(),
                        len: bytes.len(),
                    },
                    sink,
                    (&mut output as *mut Vec<u8>).cast(),
                )
            };
            assert!(status.is_ok());
            wire::decode(&output).unwrap()
        }

        fn execute(&self, name: &str, parsed: ParsedCommand) -> WireCommandOutput {
            let (_, handle, vtable) = self.commands.iter().find(|entry| entry.0 == name).unwrap();
            let input = WireCommandInput {
                wire_version: RUNTIME_WIRE_VERSION,
                session: wire::encode_session(&Session::new()).unwrap(),
                viewport_image: None,
                parsed,
                quiet: false,
                viewport_width: 800,
                viewport_height: 600,
                dynamic_settings: vec![],
                displayed_geometry: None,
                displayed_geometry_spool: None,
            };
            let bytes = wire::encode(&input).unwrap();
            let mut output = Vec::<u8>::new();
            // SAFETY: This command handle is owned by the harness; all call
            // buffers outlive the call and no optional host runtime is used.
            let status = unsafe {
                (vtable.execute.unwrap())(
                    *handle,
                    AbiU8Slice {
                        ptr: bytes.as_ptr(),
                        len: bytes.len(),
                    },
                    std::ptr::null(),
                    HostCommandRuntimeHandle(std::ptr::null_mut()),
                    sink,
                    (&mut output as *mut Vec<u8>).cast(),
                )
            };
            assert!(status.is_ok());
            wire::decode(&output).unwrap()
        }
    }

    impl Drop for AbiHarness {
        fn drop(&mut self) {
            // SAFETY: Every accepted runtime handle is destroyed exactly once;
            // the plugin code remains loaded for the duration of these tests.
            unsafe {
                if let Some(handler) = self.handler.take() {
                    (handler.vtable.destroy.unwrap())(handler.handle);
                }
                for (_, handle, vtable) in self.commands.drain(..) {
                    (vtable.destroy.unwrap())(handle);
                }
                for (handle, vtable) in self.scripts.drain(..) {
                    (vtable.destroy.unwrap())(handle);
                }
                for (handle, vtable) in self.panels.drain(..) {
                    (vtable.destroy.unwrap())(handle);
                }
            }
        }
    }

    pub(crate) fn poll_input() -> WirePollInput {
        let session = Session::new();
        WirePollInput {
            shared: WirePollSharedInput {
                wire_version: RUNTIME_WIRE_VERSION,
                scene_generation: 0,
                object_names: vec![],
                selection_names: vec![],
                pick_paths: vec![],
                camera: session.camera,
                settings: session.settings,
                clear_color: [0.0; 3],
                movie: patinae_scene::MovieStateSnapshot {
                    frame_count: 0,
                    current_frame: 0,
                    is_playing: false,
                    rock_enabled: false,
                },
                viewport_image: None,
                command_names: vec![],
                setting_names: vec![],
                dynamic_settings: vec![],
            },
            command_results: vec![],
            host_query_results: vec![],
            task_invocations: vec![],
            task_cancellations: vec![],
            plugin_dirs: vec![],
        }
    }

    #[test]
    fn abi_python_commands_and_script_prepare_work_before_host_admission() {
        let mut harness = AbiHarness::default();
        harness.register(super::register);
        for (name, args) in [
            (
                "python",
                ParsedCommand::new("python")
                    .with_arg("raise RuntimeError('must not run before admission')")
                    .with_raw_args("raise RuntimeError('must not run before admission')"),
            ),
            (
                "iterate",
                ParsedCommand::new("iterate")
                    .with_arg("all")
                    .with_arg("print(name)"),
            ),
            (
                "alter",
                ParsedCommand::new("alter").with_arg("all").with_arg("b=0"),
            ),
        ] {
            let output = harness.execute(name, args);
            assert!(output.result.is_ok());
            assert!(
                matches!(output.task_requests.as_slice(), [AsyncCommandRequest::Plugin(request)] if request.kind == "python" && request.executor.is_empty())
            );
        }
        let (handle, vtable) = harness.scripts[0];
        let input = wire::encode(&WireScriptInput {
            wire_version: RUNTIME_WIRE_VERSION,
            path: "/not/read/before/admission.py".into(),
        })
        .unwrap();
        let mut bytes = Vec::<u8>::new();
        // SAFETY: The registered script handle and input/output buffers remain live.
        let status = unsafe {
            (vtable.run.unwrap())(
                handle,
                AbiU8Slice {
                    ptr: input.as_ptr(),
                    len: input.len(),
                },
                sink,
                (&mut bytes as *mut Vec<u8>).cast(),
            )
        };
        assert!(status.is_ok());
        let output: WireScriptOutput = wire::decode(&bytes).unwrap();
        assert!(
            matches!(output.result, Ok(AsyncCommandRequest::Plugin(request)) if request.payload["path"] == "/not/read/before/admission.py")
        );
    }
}
