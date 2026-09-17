//! Sequential plugin preparation without borrowing the live application.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use super::*;
use patinae_plugin::ffi::CAPABILITY_BACKGROUND_REGISTRATION;

/// Maximum number of unattached results retained by the loader.
const PREPARED_PLUGIN_CAPACITY: usize = 1;

/// Owns plugin registrations and their library until host-thread attachment.
///
/// Legacy plugins defer registration until attachment. Fields deliberately drop
/// in this order: accepted handles first, the library containing their code last.
pub struct PreparedPlugin {
    pub(super) registration: RegistrationSink,
    pub(super) library: Library,
    registered: bool,
}

// SAFETY: Only validated ABI descriptors enter this envelope. SDK commands and
// script/format/hotkey callbacks are Send + Sync; panels and handlers are Send.
// No callback runs concurrently with transfer. Legacy envelopes contain no
// plugin objects yet. The library outlives all handles, including on cancellation.
unsafe impl Send for PreparedPlugin {}

impl PreparedPlugin {
    fn open(path: &Path) -> Result<(Self, bool), String> {
        #[cfg(target_os = "windows")]
        crate::paths::apply_deps_search_paths(path);
        // SAFETY: Native plugins are the explicit extension point. The envelope
        // retains this library until every accepted object has been destroyed.
        let library = unsafe { Library::new(path) }
            .map_err(|error| format!("Failed to load library: {error}"))?;
        let declaration = load_declaration(&library)?;
        validate_declaration(&declaration)?;
        let background = declaration.capabilities & CAPABILITY_BACKGROUND_REGISTRATION != 0;
        Ok((
            Self {
                registration: RegistrationSink::new(),
                library,
                registered: false,
            },
            background,
        ))
    }

    pub(super) fn load(path: &Path) -> Result<Self, String> {
        let (mut prepared, _) = Self::open(path)?;
        prepared.complete_registration()?;
        Ok(prepared)
    }

    pub(super) fn complete_registration(&mut self) -> Result<(), String> {
        if self.registered {
            return Ok(());
        }
        let declaration = load_declaration(&self.library)?;
        initialize_plugin(&declaration)?;
        register_plugin(&declaration, &mut self.registration)?;
        self.registered = true;
        Ok(())
    }
}

/// A startup loader event, consumed on the host thread.
pub enum PluginLoadEvent {
    /// Discovery finished; retain directories for plugin resource lookup.
    Discovered {
        directories: Vec<PathBuf>,
        total: usize,
    },
    /// One library is ready to attach, or failed preparation.
    Plugin {
        path: PathBuf,
        result: Result<Box<PreparedPlugin>, String>,
        preparation_time: Duration,
    },
    /// All attempted libraries have been acknowledged by the host.
    Finished,
}

/// Prepares plugins sequentially and waits for each host attachment.
///
/// Dropping cancels pending work without joining a native callback. Libraries
/// remain owned until that callback returns; native code cannot be preempted.
pub struct BackgroundPluginLoader {
    events: Option<Receiver<PluginLoadEvent>>,
    acknowledged: Option<Sender<()>>,
    cancelled: Arc<AtomicBool>,
}

impl BackgroundPluginLoader {
    /// Starts discovery and preparation on one dedicated thread.
    ///
    /// # Errors
    /// Returns an error if the operating system cannot create the loader thread.
    pub fn start(discovery: PluginDiscovery) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel(PREPARED_PLUGIN_CAPACITY);
        let (ack_tx, ack_rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        std::thread::Builder::new()
            .name("plugin-loader".into())
            .spawn(move || {
                let directories = discovery.standard_plugin_dirs();
                let mut paths = Vec::new();
                for directory in &directories {
                    match std::fs::read_dir(directory) {
                        Ok(entries) => paths.extend(
                            entries
                                .flatten()
                                .map(|entry| entry.path())
                                .filter(|path| is_plugin_library_path(path)),
                        ),
                        Err(error) => log::debug!("Plugin directory {directory:?}: {error}"),
                    }
                }
                if tx
                    .send(PluginLoadEvent::Discovered {
                        directories,
                        total: paths.len(),
                    })
                    .is_err()
                {
                    return;
                }
                for path in paths {
                    if worker_cancelled.load(Ordering::Acquire) {
                        return;
                    }
                    let started = Instant::now();
                    let result =
                        PreparedPlugin::open(&path).and_then(|(mut prepared, background)| {
                            if background && !worker_cancelled.load(Ordering::Acquire) {
                                prepared.complete_registration()?;
                            }
                            Ok(Box::new(prepared))
                        });
                    let preparation_time = started.elapsed();
                    if worker_cancelled.load(Ordering::Acquire) {
                        return;
                    }
                    if tx
                        .send(PluginLoadEvent::Plugin {
                            path,
                            result,
                            preparation_time,
                        })
                        .is_err()
                    {
                        return;
                    }
                    // Preserve registration order even when a legacy plugin needs
                    // main-thread registration. Never prepare the next one early.
                    if ack_rx.recv().is_err() {
                        return;
                    }
                }
                let _ = tx.send(PluginLoadEvent::Finished);
            })?;
        Ok(Self {
            events: Some(rx),
            acknowledged: Some(ack_tx),
            cancelled,
        })
    }

    /// Reads one event without waiting; disconnection signals worker failure.
    ///
    /// # Errors
    /// Returns an error if the worker stopped without a completion event.
    pub fn try_next(&self) -> Result<Option<PluginLoadEvent>, String> {
        match self
            .events
            .as_ref()
            .expect("live loader receiver")
            .try_recv()
        {
            Ok(event) => Ok(Some(event)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("Plugin loader stopped unexpectedly".into()),
        }
    }

    /// Allows preparation of the next library after host attachment or failure.
    pub fn acknowledge(&self) {
        if let Some(sender) = &self.acknowledged {
            let _ = sender.send(());
        }
    }
}

impl Drop for BackgroundPluginLoader {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.acknowledged.take();
        // A prepared object's destructor may join its own worker. Keep that
        // cleanup off the UI thread as well, without extending the event loop.
        if let Some(receiver) = self.events.take() {
            let _ = std::thread::Builder::new()
                .name("plugin-loader-cleanup".into())
                .spawn(move || drop(receiver));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patinae_settings::paths::{PathResolver, PathResolverInput};
    use std::time::Duration;

    // Compile a real, dependency-free native library against the portable ABI.
    // File gates let tests prove responsiveness without a timing-based sleep.
    struct Fixture {
        root: PathBuf,
        library: PathBuf,
    }

    impl Fixture {
        fn new(background: bool, fail: bool, incompatible: bool) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "patinae-startup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&root).unwrap();
            let library = root.join(format!("fixture.{}", std::env::consts::DLL_EXTENSION));
            let ffi = Path::new(env!("CARGO_MANIFEST_DIR")).join("../patinae-plugin/src/ffi.rs");
            let response = wire::encode(&WireCommandOutput {
                task_requests: Vec::new(),
                wire_version: RUNTIME_WIRE_VERSION,
                result: Ok(()),
                output: Vec::new(),
                actions: Vec::new(),
                session: Vec::new(),
                viewport_image: None,
                viewport_image_changed: false,
            })
            .unwrap();
            let source = format!(
                r#"
#![allow(dead_code)]
#[path = {ffi:?}] mod ffi;
use ffi::*;
const ROOT: &str = {root:?};
fn mark(name: &str) {{
    std::fs::write(std::path::Path::new(ROOT).join(name), format!("{{:?}}", std::thread::current().id())).unwrap();
}}
#[no_mangle]
pub extern "C" fn mark_host_thread() {{ mark("host-thread"); }}
unsafe extern "C" fn destroy(handle: PluginCommandHandle) -> AbiStatus {{
    // SAFETY: Exactly one host owner releases this allocation.
    drop(unsafe {{ Box::from_raw(handle.0.cast::<u8>()) }});
    mark("destroyed");
    AbiStatus::OK
}}
unsafe extern "C" fn execute(_: PluginCommandHandle, _: AbiU8Slice, _: *const HostCommandRuntimeCallbacks, _: HostCommandRuntimeHandle, sink: AbiBytesSinkFn, user: *mut core::ffi::c_void) -> AbiStatus {{
    let bytes: &[u8] = &{response:?};
    unsafe {{ sink(user, AbiU8Slice {{ ptr: bytes.as_ptr(), len: bytes.len() }}) }}
}}
unsafe extern "C" fn register(handle: HostRegistrarHandle, callbacks: *const HostCallbacks) -> AbiStatus {{
    mark("entered");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !std::path::Path::new(ROOT).join("release").exists() {{
        if std::time::Instant::now() > deadline {{ return AbiStatus::HOST_ERROR; }}
        std::thread::sleep(std::time::Duration::from_millis(1));
    }}
    // SAFETY: The host lends the callback table and registrar for this call.
    let callbacks = unsafe {{ &*callbacks }};
    unsafe {{ callbacks.register_metadata.unwrap()(handle, AbiStr::from_static("slow-fixture"), AbiStr::from_static("1"), AbiStr::EMPTY); }}
    let command = AbiCommandDescriptor {{
        handle: PluginCommandHandle(Box::into_raw(Box::new(1u8)).cast()),
        vtable: AbiCommandVTable {{ execute: Some(execute), destroy: Some(destroy) }},
        name: AbiStr::from_static("slow_fixture"), description: AbiStr::EMPTY,
        usage: AbiStr::EMPTY, arguments: AbiStr::EMPTY, help: AbiStr::EMPTY,
        aliases: AbiStrSlice::EMPTY, arg_hints: AbiU8Slice::EMPTY,
        argument_syntax: 0, runtime_requirements: 0,
    }};
    let status = unsafe {{ callbacks.register_command.unwrap()(handle, &command) }};
    if !status.is_ok() {{ unsafe {{ destroy(command.handle); }} return status; }}
    if {fail} {{ AbiStatus::HOST_ERROR }} else {{ AbiStatus::OK }}
}}
#[no_mangle]
pub static PATINAE_PLUGIN_DECLARATION: PluginDeclaration = PluginDeclaration {{
    abi_version: ABI_VERSION + {incompatible}, sdk_version: AbiStr::from_static(SDK_VERSION),
    capabilities: CAPABILITY_REGISTRATION | if {background} {{ CAPABILITY_BACKGROUND_REGISTRATION }} else {{ 0 }},
    init: None, register: Some(register),
}};
"#,
                ffi = ffi.to_string_lossy(),
                root = root.to_string_lossy(),
                incompatible = u32::from(incompatible)
            );
            let source_path = root.join("fixture.rs");
            std::fs::write(&source_path, source).unwrap();
            let output = std::process::Command::new("rustc")
                .args(["--edition=2021", "--crate-type=cdylib"])
                .arg(&source_path)
                .arg("-o")
                .arg(&library)
                .env("CARGO_PKG_VERSION", SDK_VERSION)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            Self { root, library }
        }

        fn discovery(&self) -> PluginDiscovery {
            PluginDiscovery::new(PathResolver::new(PathResolverInput {
                config_dir: Some(self.root.clone()),
                plugin_dir: Some(self.root.clone()),
                ..Default::default()
            }))
        }
        fn release(&self) {
            std::fs::write(self.root.join("release"), "").unwrap();
        }
        fn wait_for(&self, marker: &str) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !self.root.join(marker).exists() {
                assert!(Instant::now() < deadline, "missing {marker}");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn next(loader: &BackgroundPluginLoader) -> PluginLoadEvent {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(event) = loader.try_next().unwrap() {
                return event;
            }
            assert!(Instant::now() < deadline, "loader event timeout");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn dynamic_preparation_does_not_block_builtin_commands_and_waits_for_attachment() {
        let fixture = Fixture::new(true, false, false);
        let loader = BackgroundPluginLoader::start(fixture.discovery()).unwrap();
        assert!(matches!(
            next(&loader),
            PluginLoadEvent::Discovered { total: 1, .. }
        ));
        fixture.wait_for("entered");
        let mut kernel = patinae_framework::kernel::AppKernel::new();
        let mut host = PluginHost::new();
        assert!(kernel
            .execute_command("set sphere_scale, 2", false, None, (1, 1))
            .is_ok());
        assert!(!kernel.executor.registry().contains("slow_fixture"));
        assert!(loader.try_next().unwrap().is_none());
        fixture.release();
        let PluginLoadEvent::Plugin { result, .. } = next(&loader) else {
            panic!("expected plugin");
        };
        assert!(!kernel.executor.registry().contains("slow_fixture"));
        host.attach_prepared(*result.unwrap(), &mut kernel.executor)
            .unwrap();
        assert!(kernel.executor.registry().contains("slow_fixture"));
        assert_eq!(host.plugin_count(), 1);
        assert!(kernel
            .execute_command("slow_fixture", false, None, (1, 1))
            .is_ok());
        assert!(loader.try_next().unwrap().is_none());
        loader.acknowledge();
        assert!(matches!(next(&loader), PluginLoadEvent::Finished));
        drop(host);
        drop(kernel);
        fixture.wait_for("destroyed");
    }

    #[test]
    fn legacy_registration_runs_only_on_attaching_thread() {
        let fixture = Fixture::new(false, false, false);
        let loader = BackgroundPluginLoader::start(fixture.discovery()).unwrap();
        assert!(matches!(next(&loader), PluginLoadEvent::Discovered { .. }));
        let PluginLoadEvent::Plugin { result, .. } = next(&loader) else {
            panic!("expected plugin");
        };
        assert!(!fixture.root.join("entered").exists());
        fixture.release();
        // Rust ThreadId counters are DLL-local. Ask the same DLL to record
        // the attaching thread instead of comparing IDs from two runtimes.
        // SAFETY: This test owns the fixture and its exported function ABI.
        let probe = unsafe { Library::new(&fixture.library).unwrap() };
        // SAFETY: The fixture defines this exact no-argument symbol.
        unsafe { probe.get::<extern "C" fn()>(b"mark_host_thread").unwrap()() };
        let mut executor = CommandExecutor::new();
        let mut host = PluginHost::new();
        host.attach_prepared(*result.unwrap(), &mut executor)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("entered")).unwrap(),
            std::fs::read_to_string(fixture.root.join("host-thread")).unwrap()
        );
        loader.acknowledge();
        assert!(matches!(next(&loader), PluginLoadEvent::Finished));
        drop(host);
        drop(executor);
        fixture.wait_for("destroyed");
    }

    #[test]
    fn partial_registration_failure_destroys_accepted_handles() {
        let fixture = Fixture::new(true, true, false);
        fixture.release();
        let loader = BackgroundPluginLoader::start(fixture.discovery()).unwrap();
        assert!(matches!(next(&loader), PluginLoadEvent::Discovered { .. }));
        let PluginLoadEvent::Plugin { result, .. } = next(&loader) else {
            panic!("expected plugin");
        };
        assert!(result.is_err());
        fixture.wait_for("destroyed");
        loader.acknowledge();
        assert!(matches!(next(&loader), PluginLoadEvent::Finished));
    }

    #[test]
    fn cancellation_does_not_wait_for_registration_and_destroys_late_result() {
        let fixture = Fixture::new(true, false, false);
        let loader = BackgroundPluginLoader::start(fixture.discovery()).unwrap();
        assert!(matches!(next(&loader), PluginLoadEvent::Discovered { .. }));
        fixture.wait_for("entered");
        drop(loader); // Must return while the registration gate is still closed.
        fixture.release();
        fixture.wait_for("destroyed");
    }

    #[test]
    fn dropping_unattached_preparation_releases_handles_before_library() {
        let fixture = Fixture::new(true, false, false);
        fixture.release();
        let prepared = PreparedPlugin::load(&fixture.library).unwrap();
        assert!(!fixture.root.join("destroyed").exists());
        drop(prepared);
        fixture.wait_for("destroyed");
    }

    #[test]
    fn failed_library_does_not_prevent_loading_other_libraries() {
        let valid = Fixture::new(true, false, false);
        let invalid = Fixture::new(true, false, true);
        valid.release();
        std::fs::copy(
            &invalid.library,
            valid
                .root
                .join(format!("invalid.{}", std::env::consts::DLL_EXTENSION)),
        )
        .unwrap();
        let loader = BackgroundPluginLoader::start(valid.discovery()).unwrap();
        assert!(matches!(
            next(&loader),
            PluginLoadEvent::Discovered { total: 2, .. }
        ));
        let mut host = PluginHost::new();
        let mut executor = CommandExecutor::new();
        let mut errors = 0;
        for _ in 0..2 {
            let PluginLoadEvent::Plugin { result, .. } = next(&loader) else {
                panic!("expected plugin");
            };
            match result {
                Ok(prepared) => {
                    host.attach_prepared(*prepared, &mut executor).unwrap();
                }
                Err(_) => errors += 1,
            }
            loader.acknowledge();
        }
        assert!(matches!(next(&loader), PluginLoadEvent::Finished));
        assert_eq!(errors, 1);
        assert!(executor.registry().contains("slow_fixture"));
        drop(host);
        drop(executor);
        valid.wait_for("destroyed");
    }

    #[test]
    fn incompatible_library_is_rejected_before_registration() {
        let fixture = Fixture::new(true, false, true);
        assert!(PreparedPlugin::load(&fixture.library).is_err());
        assert!(!fixture.root.join("entered").exists());
    }
}
