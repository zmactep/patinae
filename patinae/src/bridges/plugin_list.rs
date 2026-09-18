//! Editable plugin allowlist backed by live library identities.

use std::{
    cell::RefCell,
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
};

use crate::{AppWindow, PluginListRow, PluginListState};
use patinae_cmd::LoadedPluginCapability;
use patinae_plugin_host::{
    plugin_manifest_path, read_plugin_manifest, save_plugin_manifest, BackgroundPluginLoader,
    PluginHost, PluginLoadEvent,
};
use slint::{ComponentHandle, ModelRc, VecModel};

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    metadata: Option<LoadedPluginCapability>,
}

fn identity(path: &Path) -> PathBuf {
    path.canonicalize()
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn loaded_entries(host: &PluginHost) -> Vec<Entry> {
    host.loaded_libraries()
        .map(|(path, metadata)| Entry {
            path: path.to_path_buf(),
            metadata: Some(LoadedPluginCapability {
                name: metadata.name.clone(),
                version: metadata.version.clone(),
                description: metadata.description.clone(),
            }),
        })
        .collect()
}

#[derive(Default)]
pub(crate) struct PluginList {
    publish_pending: bool,
    pending_apply: Option<Vec<PathBuf>>,
    loader: Option<BackgroundPluginLoader>,
    target: PathBuf,
    entries: Vec<Entry>,
    removed: HashSet<PathBuf>,
    loaded: Vec<Entry>,
    damaged: bool,
    status: String,
    save_error: String,
    dirty: bool,
}

impl PluginList {
    fn read(directories: &[PathBuf], loaded: &[Entry]) -> Self {
        let fallback = directories
            .first()
            .cloned()
            .unwrap_or_default()
            .join("plugins.toml");
        let mut list = Self {
            target: fallback,
            loaded: loaded.to_vec(),
            ..Self::default()
        };
        match plugin_manifest_path(directories) {
            Ok(Some(path)) => list.target = path,
            Ok(None) => {
                list.entries = loaded.to_vec();
                list.status = "TOML file does not exist.".into();
                return list;
            }
            Err(error) => {
                list.damaged = true;
                list.status = format!("Cannot read TOML file: {error}");
                return list;
            }
        }
        match read_plugin_manifest(directories) {
            Ok(Some(document)) => {
                list.entries = loaded.to_vec();
                let loaded_paths: HashSet<_> =
                    loaded.iter().map(|entry| identity(&entry.path)).collect();
                for entry in document.entries {
                    let path = identity(&entry.resolved_path);
                    if !loaded_paths.contains(&path) {
                        list.entries.push(Entry {
                            path,
                            metadata: None,
                        });
                    }
                }
            }
            Ok(None) => {
                list.entries = loaded.to_vec();
                list.status = "TOML file does not exist.".into();
            }
            Err(_) => {
                list.damaged = true;
                list.status = "TOML file is damaged or cannot be read.".into();
            }
        }
        list
    }

    fn refresh_loaded(&mut self, loaded: &[Entry]) -> bool {
        if self.loaded == loaded {
            return false;
        }
        self.loaded = loaded.to_vec();
        if self.damaged {
            return false;
        }
        for entry in &mut self.entries {
            entry.metadata = None;
        }
        for plugin in loaded {
            let key = identity(&plugin.path);
            if self.removed.contains(&key) {
                continue;
            }
            let mut found = false;
            for entry in &mut self.entries {
                if identity(&entry.path) == key {
                    entry.metadata = plugin.metadata.clone();
                    found = true;
                }
            }
            if !found {
                self.entries.push(plugin.clone());
            }
        }
        true
    }

    fn remove(&mut self, index: usize) {
        if index >= self.entries.len() {
            return;
        }
        let entry = self.entries.remove(index);
        if !self
            .entries
            .iter()
            .any(|other| identity(&other.path) == identity(&entry.path))
        {
            self.removed.insert(identity(&entry.path));
        }
        self.dirty = true;
        self.save_error.clear();
        if self.status.starts_with("Saved.") {
            self.status.clear();
        }
    }

    fn add(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let mut errors = Vec::new();
        for path in paths {
            if !patinae_plugin_host::is_plugin_library_path(&path)
                || !path.is_file()
                || path.to_str().is_none()
            {
                errors.push(format!(
                    "Cannot add {}: select an existing .{} library with a Unicode path.",
                    path.display(),
                    std::env::consts::DLL_EXTENSION
                ));
                continue;
            }
            let path = identity(&path);
            if self
                .entries
                .iter()
                .any(|entry| identity(&entry.path) == path)
            {
                continue;
            }
            self.removed.remove(&path);
            let metadata = self
                .loaded
                .iter()
                .find(|entry| identity(&entry.path) == path)
                .and_then(|entry| entry.metadata.clone());
            self.entries.push(Entry { path, metadata });
            self.dirty = true;
            if self.status.starts_with("Saved.") {
                self.status.clear();
            }
        }
        self.save_error = errors.join("\n");
    }

    fn save(&mut self) {
        let paths: Vec<_> = self
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        match save_plugin_manifest(&self.target, &paths) {
            Ok(()) => {
                self.damaged = false;
                self.dirty = false;
                self.save_error.clear();
                self.status = "Saved. Applying changes…".into();
                self.pending_apply = Some(paths);
            }
            Err(error) => self.save_error = error,
        }
    }

    fn applying(&self) -> bool {
        self.pending_apply.is_some() || self.loader.is_some()
    }

    /// Apply a saved snapshot between host callbacks, even after the dialog closes.
    pub(crate) fn tick(
        &mut self,
        host: &mut PluginHost,
        kernel: &mut patinae_framework::kernel::AppKernel,
    ) -> bool {
        if !self.applying() {
            return false;
        }
        let mut changed = false;
        if let Some(paths) = self.pending_apply.take() {
            self.publish_pending = true;
            let desired: HashSet<_> = paths.iter().map(|path| identity(path)).collect();
            let current: Vec<_> = host
                .loaded_libraries()
                .map(|(path, _)| path.to_path_buf())
                .collect();
            for path in &current {
                if !desired.contains(&identity(path)) {
                    changed |= host.unload_library(path, &mut kernel.executor, &kernel.tasks);
                }
            }
            let mut seen: HashSet<_> = current.into_iter().map(|path| identity(&path)).collect();
            let additions: Vec<_> = paths
                .into_iter()
                .filter(|path| seen.insert(identity(path)))
                .collect();
            if !additions.is_empty() {
                match BackgroundPluginLoader::start_paths(additions) {
                    Ok(loader) => self.loader = Some(loader),
                    Err(error) => self.save_error = format!("Cannot start plugin loader: {error}"),
                }
            }
        }
        if let Some(loader) = &self.loader {
            match loader.try_next() {
                Ok(Some(PluginLoadEvent::Plugin { path, result, .. })) => {
                    self.publish_pending = true;
                    match result
                        .and_then(|prepared| host.attach_prepared(*prepared, &mut kernel.executor))
                    {
                        Ok(_) => changed = true,
                        Err(error) => {
                            if !self.save_error.is_empty() {
                                self.save_error.push('\n');
                            }
                            self.save_error
                                .push_str(&format!("{}: {error}", path.display()));
                        }
                    }
                    loader.acknowledge();
                }
                Ok(Some(PluginLoadEvent::Finished)) => self.loader = None,
                Err(error) | Ok(Some(PluginLoadEvent::DiscoveryError { error })) => {
                    self.save_error = error;
                    self.loader = None;
                }
                Ok(None | Some(PluginLoadEvent::Discovered { .. })) => {}
            }
        }
        self.publish_pending |= self.refresh_loaded(&loaded_entries(host));
        if !self.applying() && self.status == "Saved. Applying changes…" {
            self.publish_pending = true;
            self.status = if self.save_error.is_empty() {
                "Saved. Changes applied.".into()
            } else {
                "Saved. Some plugins could not be loaded.".into()
            };
        }
        changed
    }

    fn rows(&self) -> Vec<PluginListRow> {
        self.entries
            .iter()
            .map(|entry| {
                let (title, description) = if let Some(metadata) = &entry.metadata {
                    (
                        format!("{} v{}", metadata.name, metadata.version),
                        metadata.description.clone(),
                    )
                } else {
                    (
                        entry
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        String::new(),
                    )
                };
                PluginListRow {
                    title: title.into(),
                    description: description.into(),
                    path: entry.path.display().to_string().into(),
                    missing: entry.metadata.is_none(),
                }
            })
            .collect()
    }

    fn publish(&self, window: &AppWindow) {
        let state = window.global::<PluginListState>();
        state.set_applying(self.applying());
        state.set_source(self.target.display().to_string().into());
        state.set_status(self.status.clone().into());
        state.set_error(self.save_error.clone().into());
        state.set_damaged(self.damaged);
        state.set_dirty(self.dirty);
        state.set_rows(ModelRc::new(VecModel::from(self.rows())));
    }

    pub(crate) fn sync(&mut self, window: &AppWindow, host: &PluginHost, status: Option<String>) {
        let state = window.global::<PluginListState>();
        if !state.get_visible() {
            return;
        }
        let status = status.unwrap_or_else(|| {
            if self.applying() {
                "Applying plugin changes…".into()
            } else {
                String::new()
            }
        });
        if state.get_loading_status() != status {
            state.set_loading_status(status.into());
        }
        if self.refresh_loaded(&loaded_entries(host)) || self.publish_pending {
            self.publish(window);
            self.publish_pending = false;
        }
    }
}

pub(crate) fn setup_callbacks(app: Rc<RefCell<crate::app::App>>, window: &AppWindow) {
    let add_app = app.clone();
    let weak = window.as_weak();
    window.global::<PluginListState>().on_add(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if window.global::<PluginListState>().get_picking()
            || window.global::<PluginListState>().get_applying()
        {
            return;
        }
        window.global::<PluginListState>().set_picking(true);
        let app = add_app.clone();
        let weak = window.as_weak();
        // AppKit runs a nested event loop. Defer past the Slint callback and
        // hold no App borrow while the native picker is open.
        if let Err(error) = slint::spawn_local(async move {
            let Some(window) = weak.upgrade() else {
                return;
            };
            #[cfg(target_os = "macos")]
            let selected: Result<Vec<PathBuf>, String> =
                Ok(crate::macos::open_plugin_paths().unwrap_or_default());
            #[cfg(not(target_os = "macos"))]
            let selected: Result<Vec<PathBuf>, String> =
                Err("Plugin file picker is unavailable on this platform.".into());
            window.global::<PluginListState>().set_picking(false);
            let mut app = app.borrow_mut();
            match selected {
                Ok(paths) => app.plugin_list.add(paths),
                Err(error) => app.plugin_list.save_error = error,
            }
            app.plugin_list.publish(&window);
        }) {
            window.global::<PluginListState>().set_picking(false);
            let mut app = add_app.borrow_mut();
            app.plugin_list.save_error = format!("Cannot open plugin picker: {error}");
            app.plugin_list.publish(&window);
        }
    });
    let weak = window.as_weak();
    let open_app = app.clone();
    window.global::<PluginListState>().on_open(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let mut app = open_app.borrow_mut();
        if !app.plugin_list.applying() {
            app.plugin_list = PluginList::read(
                &patinae_plugin_host::standard_plugin_dirs(),
                &loaded_entries(&app.plugins),
            );
        }
        app.plugin_list.publish(&window);
        window.global::<PluginListState>().set_visible(true);
        app.sync_plugin_list(&window);
    });
    let weak = window.as_weak();
    let remove_app = app.clone();
    window.global::<PluginListState>().on_remove(move |index| {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let mut app = remove_app.borrow_mut();
        if app.plugin_list.applying() {
            return;
        }
        app.plugin_list.remove(index);
        app.plugin_list.publish(&window);
    });
    let weak = window.as_weak();
    window.global::<PluginListState>().on_save(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let mut app = app.borrow_mut();
        if app.plugin_list.applying()
            || !window
                .global::<PluginListState>()
                .get_loading_status()
                .is_empty()
        {
            return;
        }
        app.plugin_list.save();
        app.plugin_list.publish(&window);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "patinae-plugin-list-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, text: &str) {
            std::fs::write(self.0.join("plugins.toml"), text).unwrap();
        }

        fn read(&self, loaded: &[Entry]) -> PluginList {
            PluginList::read(std::slice::from_ref(&self.0), loaded)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn runtime_model_uses_successful_real_host_registrations_without_panels() {
        let fixture = Fixture::new();
        let ffi =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../crates/patinae-plugin/src/ffi.rs");
        let mut kernel = patinae_framework::kernel::AppKernel::new();
        let mut host = patinae_plugin_host::PluginHost::new();
        let mut list = fixture.read(&loaded_entries(&host));
        // Use the same standalone ABI fixture approach as loader/startup tests.
        for fail in [true, false] {
            let source = format!(
                r#"
#![allow(dead_code)]
#[path = {ffi:?}] mod ffi;
use ffi::*;
unsafe extern "C" fn register(handle: HostRegistrarHandle, callbacks: *const HostCallbacks) -> AbiStatus {{
    // SAFETY: PluginHost lends a valid callback table and registrar for this call.
    let callbacks = unsafe {{ &*callbacks }};
    // SAFETY: The borrowed registrar is live and all strings have static storage.
    let status = unsafe {{ callbacks.register_metadata.unwrap()(handle,
        AbiStr::from_static("headless"), AbiStr::from_static("1.2"),
        AbiStr::from_static("No UI panels")) }};
    if {fail} {{ AbiStatus::HOST_ERROR }} else {{ status }}
}}
#[no_mangle]
pub static PATINAE_PLUGIN_DECLARATION: PluginDeclaration = PluginDeclaration {{
    abi_version: ABI_VERSION, sdk_version: AbiStr::from_static(SDK_VERSION),
    capabilities: CAPABILITY_REGISTRATION, init: None, register: Some(register),
}};
"#,
                ffi = ffi.to_string_lossy()
            );
            let source_path = fixture.0.join(format!("fixture_{fail}.rs"));
            let library = source_path.with_extension(std::env::consts::DLL_EXTENSION);
            std::fs::write(&source_path, source).unwrap();
            let output = std::process::Command::new("rustc")
                .args(["--edition=2021", "--crate-type=cdylib"])
                .arg(&source_path)
                .arg("-o")
                .arg(&library)
                .env("CARGO_PKG_VERSION", env!("CARGO_PKG_VERSION"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                host.load_library(&library, &mut kernel.executor).is_err(),
                fail
            );
            assert_eq!(list.refresh_loaded(&loaded_entries(&host)), !fail);
            assert_eq!(list.rows().len(), usize::from(!fail));
        }
        assert_eq!(host.plugin_count(), 1);
        assert_eq!(list.rows()[0].title, "headless v1.2");
        assert_eq!(list.rows()[0].description, "No UI panels");
        assert_eq!(
            list.rows()[0].path,
            list.entries[0].path.display().to_string()
        );
        assert!(list.entries[0].path.is_absolute());
        list.save();
        assert!(list.save_error.is_empty());
        let document = read_plugin_manifest(std::slice::from_ref(&fixture.0))
            .unwrap()
            .unwrap();
        assert_eq!(document.entries[0].resolved_path, list.entries[0].path);
        apply_saved(&mut list, &mut host, &mut kernel);
        assert_eq!(host.plugin_count(), 1);
        let active = list.entries[0].path.clone();
        list.remove(0);
        list.save();
        assert_eq!(host.plugin_count(), 1, "saving only queues application");
        apply_saved(&mut list, &mut host, &mut kernel);
        assert_eq!(host.plugin_count(), 0);
        assert!(kernel.executor.loaded_plugin_capabilities().is_empty());

        assert!(read_plugin_manifest(std::slice::from_ref(&fixture.0))
            .unwrap()
            .unwrap()
            .entries
            .is_empty());
        let broken = fixture
            .0
            .join("fixture_true")
            .with_extension(std::env::consts::DLL_EXTENSION);
        list.add(vec![broken.clone(), active.clone()]);
        list.save();
        apply_saved(&mut list, &mut host, &mut kernel);
        assert_eq!(host.plugin_count(), 1);
        assert!(list.rows()[0].missing);
        assert!(!list.rows()[1].missing);
        assert!(list.save_error.contains("register failed"));
        assert_eq!(kernel.executor.loaded_plugin_capabilities().len(), 1);
        // Reapplying retries failures without reattaching successful libraries.
        list.save();
        apply_saved(&mut list, &mut host, &mut kernel);
        assert_eq!(host.plugin_count(), 1);
        list.remove(0);
        list.save();
        apply_saved(&mut list, &mut host, &mut kernel);
        assert!(list.save_error.is_empty());
        assert_eq!(list.status, "Saved. Changes applied.");
        let old_target = list.target.clone();
        list.target = fixture.0.clone(); // A directory cannot be replaced by a manifest.
        list.remove(0);
        list.save();
        assert!(!list.save_error.is_empty());
        assert!(!list.applying());
        assert!(!list.tick(&mut host, &mut kernel));
        assert_eq!(host.plugin_count(), 1, "failed persistence must not unload");
        list.target = old_target;
        list.save();
        apply_saved(&mut list, &mut host, &mut kernel);
        assert_eq!(host.plugin_count(), 0);
    }

    fn apply_saved(
        list: &mut PluginList,
        host: &mut PluginHost,
        kernel: &mut patinae_framework::kernel::AppKernel,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while list.applying() {
            assert!(
                std::time::Instant::now() < deadline,
                "plugin application timed out"
            );
            list.tick(host, kernel);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn loaded(path: PathBuf) -> Entry {
        Entry {
            path,
            metadata: Some(LoadedPluginCapability {
                name: "plugin".into(),
                version: "1".into(),
                description: "Loaded library".into(),
            }),
        }
    }

    #[test]
    fn add_multiple_libraries_deduplicates_and_only_save_writes_manifest() {
        let fixture = Fixture::new();
        let first = fixture
            .0
            .join(format!("first.{}", std::env::consts::DLL_EXTENSION));
        let second = fixture
            .0
            .join(format!("second.{}", std::env::consts::DLL_EXTENSION));
        for path in [&first, &second] {
            std::fs::write(path, "library fixture").unwrap();
        }
        let mut list = fixture.read(&[]);
        list.add(vec![first.clone(), second.clone(), first.clone()]);
        assert_eq!(list.entries.len(), 2);
        assert!(list.rows().iter().all(|row| row.missing));
        assert!(list.dirty);
        assert!(!list.target.exists());
        list.save();
        assert!(list.save_error.is_empty());
        let document = read_plugin_manifest(std::slice::from_ref(&fixture.0))
            .unwrap()
            .unwrap();
        assert_eq!(
            document
                .entries
                .iter()
                .map(|entry| &entry.resolved_path)
                .collect::<Vec<_>>(),
            vec![&identity(&first), &identity(&second)]
        );
        list.add(vec![first]);
        assert!(!list.dirty);
        assert_eq!(list.entries.len(), 2);
    }

    #[test]
    fn add_restores_removed_loaded_plugin_and_cancel_preserves_draft() {
        let fixture = Fixture::new();
        let path = fixture
            .0
            .join(format!("loaded.{}", std::env::consts::DLL_EXTENSION));
        std::fs::write(&path, "library fixture").unwrap();
        let mut list = fixture.read(&[loaded(identity(&path))]);
        list.remove(0);
        list.add(vec![path.clone()]);
        assert!(!list.rows()[0].missing);
        assert!(!list.removed.contains(&identity(&path)));
        list.save_error = "Existing error".into();
        let before = list.entries.clone();
        list.add(Vec::new());
        assert_eq!(list.entries, before);
        assert_eq!(list.save_error, "Existing error");
        assert!(list.dirty);
        assert!(!list.target.exists());
    }

    #[test]
    fn add_rejects_non_libraries_missing_files_and_directories() {
        let fixture = Fixture::new();
        let text = fixture.0.join("file.txt");
        std::fs::write(&text, "text").unwrap();
        let directory = fixture
            .0
            .join(format!("directory.{}", std::env::consts::DLL_EXTENSION));
        std::fs::create_dir(&directory).unwrap();
        let mut list = fixture.read(&[]);
        list.add(vec![
            text,
            directory,
            fixture
                .0
                .join(format!("missing.{}", std::env::consts::DLL_EXTENSION)),
        ]);
        assert!(list.entries.is_empty());
        assert!(!list.dirty);
        assert_eq!(list.save_error.lines().count(), 3);
        assert!(!list.target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn add_deduplicates_symlink_aliases() {
        let fixture = Fixture::new();
        let path = fixture
            .0
            .join(format!("library.{}", std::env::consts::DLL_EXTENSION));
        let alias = fixture
            .0
            .join(format!("alias.{}", std::env::consts::DLL_EXTENSION));
        std::fs::write(&path, "library fixture").unwrap();
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        let mut list = fixture.read(&[]);
        list.add(vec![path, alias]);
        assert_eq!(list.entries.len(), 1);
    }

    #[test]
    fn missing_relative_paths_are_saved_without_rebasing_twice() {
        let path = identity(Path::new("plugins/missing-library-for-editor-test.dylib"));
        assert!(path.is_absolute());
        assert_eq!(
            path,
            std::env::current_dir()
                .unwrap()
                .join("plugins/missing-library-for-editor-test.dylib")
        );
    }

    #[test]
    fn merges_live_plugins_with_only_unloaded_manifest_entries() {
        let fixture = Fixture::new();
        let path = fixture.0.join("active.dylib");
        std::fs::write(&path, "fixture").unwrap();
        fixture.write("[[plugin]]\npath = './active.dylib'\n[[plugin]]\npath = 'missing.dylib'");
        let active = loaded(path);
        let extra = loaded(fixture.0.join("other/active.dylib"));
        let list = fixture.read(&[active, extra]);
        let rows = list.rows();
        assert_eq!(rows.len(), 3);
        assert!(!rows[0].missing);
        assert!(!rows[1].missing);
        assert!(rows[2].missing);
        assert_eq!(rows[2].title, "missing.dylib");
    }

    #[test]
    fn missing_and_empty_files_show_loaded_but_damaged_file_stays_empty() {
        let fixture = Fixture::new();
        let active = loaded(fixture.0.join("active.dylib"));
        let list = fixture.read(std::slice::from_ref(&active));
        assert_eq!(list.rows().len(), 1);
        assert!(list.status.contains("does not exist"));
        fixture.write("");
        assert_eq!(fixture.read(std::slice::from_ref(&active)).rows().len(), 1);
        fixture.write("[");
        let mut list = fixture.read(&[]);
        assert!(list.damaged);
        assert!(list.status.contains("damaged"));
        assert!(!list.refresh_loaded(std::slice::from_ref(&active)));
        assert!(list.rows().is_empty());
        list.save();
        assert!(list.save_error.is_empty());
        assert!(!list.damaged);
        assert!(read_plugin_manifest(std::slice::from_ref(&fixture.0))
            .unwrap()
            .unwrap()
            .entries
            .is_empty());
    }

    #[test]
    fn deletion_is_a_draft_and_startup_updates_do_not_restore_removed_rows() {
        let fixture = Fixture::new();
        fixture.write("[[plugin]]\npath = 'late.dylib'\n[[plugin]]\npath = 'missing.dylib'");
        let original = std::fs::read(fixture.0.join("plugins.toml")).unwrap();
        let mut list = fixture.read(&[]);
        list.remove(0);
        assert!(list.dirty);
        assert_eq!(
            std::fs::read(fixture.0.join("plugins.toml")).unwrap(),
            original
        );
        let live = vec![
            loaded(fixture.0.join("late.dylib")),
            loaded(fixture.0.join("new.dylib")),
        ];
        assert!(list.refresh_loaded(&live));
        assert_eq!(list.rows().len(), 2);
        assert_eq!(list.rows()[0].title, "missing.dylib");
        assert!(!list.rows()[1].missing);
        list.save();
        assert!(list.save_error.is_empty());
        assert!(!list.dirty);
        let saved = read_plugin_manifest(std::slice::from_ref(&fixture.0))
            .unwrap()
            .unwrap();
        assert_eq!(saved.entries.len(), 2);
        assert!(saved
            .entries
            .iter()
            .all(|entry| !entry.path.ends_with("late.dylib")));
        // Reopening shows the real runtime again, including the still-loaded plugin.
        assert_eq!(fixture.read(&live).rows().len(), 3);
    }

    #[test]
    fn startup_registration_updates_missing_row_and_save_failure_keeps_draft() {
        let fixture = Fixture::new();
        fixture.write("[[plugin]]\npath = 'late.dylib'");
        let mut list = fixture.read(&[]);
        assert!(list.rows()[0].missing);
        assert!(list.refresh_loaded(&[loaded(fixture.0.join("late.dylib"))]));
        assert_eq!(list.rows().len(), 1);
        assert!(!list.rows()[0].missing);
        list.remove(0);
        std::fs::remove_file(&list.target).unwrap();
        std::fs::create_dir(&list.target).unwrap();
        list.save();
        assert!(!list.save_error.is_empty());
        assert!(list.dirty);
        assert!(list.entries.is_empty());
        assert!(list.target.is_dir());
    }
}
